use std::sync::Arc;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, RwLock};
use tokio::io::BufReader;
use std::env;
use tracing::{info, warn, error};
use tracing_subscriber::{EnvFilter, fmt};
use tracing_appender::rolling;

mod config;
mod ws_server;

use tracing_subscriber::prelude::*;

use crate::ws_server::{start_ws_server, start_http_server, WsContext};
use crate::ws_server::questdb::{QuestDb, QuestDbConfig};
use crate::ws_server::OptionalDb;

fn init_logging() -> anyhow::Result<()> {
    // Log a archivo rotativo diario en ./logs/artheris.log.YYYY-MM-DD
    let file_appender = rolling::daily("./logs", "artheris.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Consola + archivo
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stdout) // consola
                .with_target(false)
                .with_level(true)
        )
        .with(
            fmt::layer()
                .with_writer(non_blocking) // archivo
                .with_target(false)
                .with_level(true)
        )
        .with(EnvFilter::from_default_env().add_directive("debug".parse()?))
        .try_init()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    info!("🚀 Iniciando Artheris UDP/Web");
    Ok(())
}

fn extract_numeric_record_and_time(
    v: &serde_json::Value,
) -> Option<(serde_json::Map<String, serde_json::Value>, Option<String>)> {
    use serde_json::{Map, Value};

    // 1) localiza objeto con los datos (puede venir en v["payload"] o al tope)
    let obj_opt = if let Some(obj) = v.as_object() {
        if let Some(payload) = obj.get("payload").and_then(|p| p.as_object()) {
            Some(payload)
        } else {
            Some(obj)
        }
    } else {
        None
    };

    let obj = obj_opt?;

    // 2) filtra numéricos y detecta posible campo tiempo
    let mut fields = Map::new();
    let mut ts_field: Option<String> = None;

    for (k, val) in obj {
        // detecta campo tiempo común
        let is_time_key = matches!(
            k.as_str(),
            "time" | "timestamp" | "ts" | "Time" | "Timestamp" | "TS"
        );

        if is_time_key {
            // puede ser número epoch(ns/ms/s) o string ISO
            ts_field = Some(k.clone());
            continue;
        }

        if val.is_number() {
            fields.insert(k.clone(), val.clone());
        }
    }

    if fields.is_empty() {
        return None;
    }
    Some((fields, ts_field))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(e) = init_logging() {
        eprintln!("❌ No se pudo inicializar el logging: {e}");
        return Err(e);
    }

    let questdb_config = QuestDbConfig {
        host: std::env::var("QUESTDB_HOST").unwrap_or_else(|_| "127.0.0.1".into()), // <-- antes "localhost"
        port: std::env::var("QUESTDB_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8812),
        user: std::env::var("QUESTDB_USER").unwrap_or_else(|_| "admin".into()),
        password: std::env::var("QUESTDB_PASSWORD").unwrap_or_else(|_| "quest".into()),
        database: std::env::var("QUESTDB_DB").unwrap_or_else(|_| "qdb".into()),
        table_name: Some("flight_telemetry".to_string()),
        time_col: Some("timestamp".to_string()),
    };    

    info!("🔧 Configuración de QuestDB: host={} port={}", questdb_config.host, questdb_config.port);

    let qdb = {
        let db = OptionalDb::new(questdb_config.clone());

        match QuestDb::connect(questdb_config.clone()).await {
            Ok(_conn) => {
                info!("✅ Conectado a QuestDB");
                db
            }
            Err(e) => {
                warn!("⚠️  No se pudo conectar a QuestDB al inicio: {e}. Se intentará bajo demanda.");
                db
            }
        }
    };

    // 🔹 Estado compartido
    let current_flight_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
    let last_config: Arc<RwLock<Option<serde_json::Value>>> = Arc::new(RwLock::new(None));

    // Canal broadcast para WS
    let (tx, _) = broadcast::channel::<String>(100);

    // --------- UDP ----------
    const LOCAL_PORT: u16 = 8889;
    const REMOTE_IP: &str = "192.168.1.50";
    const REMOTE_PORT: u16 = 8888;

    let local_addr = format!("0.0.0.0:{}", LOCAL_PORT);
    let remote_addr: SocketAddr = format!("{}:{}", REMOTE_IP, REMOTE_PORT).parse().unwrap();

    // Bind UDP local
    let socket = Arc::new(UdpSocket::bind(local_addr.clone()).await?);
    println!("✅ UDP listening on {}", local_addr);

    // 🔹 Contexto compartido
    let ws_ctx = WsContext {
        tx: tx.clone(),
        esp32_socket: Some(socket.clone()),
        remote_addr,
        questdb: qdb.clone(),                 // ahora es ws_server::server::OptionalDb
        flight_id: current_flight_id.clone(),
        last_config: last_config.clone(),
    };

    // WS server
    let _ws_server = tokio::spawn({
        let ctx = ws_ctx.clone();
        async move {
            info!("🔌 Iniciando servidor WebSocket en ws://0.0.0.0:9001");
            start_ws_server(ctx).await;
            info!("✅ Servidor WebSocket detenido");
        }
    });

    // HTTP server
    let _http_server = tokio::spawn({
        let ctx = ws_ctx.clone();
        async move {
            info!("🌍 Iniciando servidor HTTP en http://0.0.0.0:3000");
            if let Err(e) = start_http_server(ctx).await {
                error!("❌ Error en el servidor HTTP: {}", e);
            }
        }
    });

    {
        let socket_recv = Arc::clone(&socket);
        let tx_udp = tx.clone();
        let qdb_writer = qdb.clone();
        let flight_state = current_flight_id.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                match socket_recv.recv_from(&mut buf).await {
                    Ok((len, _src)) => {
                        if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                            let (to_ws, to_store) = match serde_json::from_str::<serde_json::Value>(text) {
                                Ok(v) => match v.get("type").and_then(|t| t.as_str()) {
                                    Some("ack") | Some("telemetry") => (v.to_string(), Some(v)),
                                    _ => {
                                        let wrapped = serde_json::json!({ "type":"telemetry", "payload": v });
                                        (wrapped.to_string(), Some(wrapped))
                                    }
                                },
                                Err(_) => {
                                    let wrapped = serde_json::json!({ "type":"telemetry", "payload": text });
                                    (wrapped.to_string(), Some(wrapped))
                                }
                            };

                            let _ = tx_udp.send(to_ws);

                            if let Some(flog) = to_store {
                                // 1) flight_id activo
                                let fid_opt = { flight_state.read().await.clone() };
                                if let Some(ref fid) = fid_opt {
                                    // 2) Guarda crudo en flight_logs (como antes)
                                    if let Err(e) = qdb_writer.insert_flight_log(&fid, &flog.to_string()).await {
                                        error!("❌ [flight_logs] insert_flight_log falló: {e}");
                                    }
                            
                                    // 3) EXTRAER registro numérico + campo tiempo y mandar a ILP → flight_telemetry
                                    match extract_numeric_record_and_time(&flog) {
                                        Some((record_obj, ts_field_opt)) => {
                                            let rec_json = serde_json::Value::Object(record_obj);
                                            match qdb_writer
                                                .ingest_telemetry_batch(
                                                    fid,
                                                    "1",            // schema_version
                                                    None,           // mode (si quieres, pásalo desde tu payload)
                                                    std::slice::from_ref(&rec_json),
                                                    ts_field_opt.as_deref(),
                                                )
                                                .await
                                            {
                                                Ok(n) => {
                                                    // n = líneas ILP enviadas (debería ser 1)
                                                    tracing::info!(
                                                        "✅ [flight_telemetry] ILP ok: inserted={} flight_id={}",
                                                        n, fid
                                                    );
                                                }
                                                Err(e) => {
                                                    // Cuando esto falle, verás el porqué aquí
                                                    tracing::error!("❌ [flight_telemetry] ILP ingest falló: {}", e);
                                                }
                                            }
                                        }
                                        None => {
                                            // No pudimos armar un objeto numérico (tal vez el payload no es el esperado)
                                            tracing::debug!(
                                                "ℹ️ extract_numeric_record_and_time: no numéricos/forma no soportada; flog={}",
                                                flog
                                            );
                                        }
                                    }
                                } else {
                                    //tracing::debug!("ℹ️ Sin flight_id activo: ignorando insert a flight_telemetry");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("❌ UDP recv error: {e}");
                        break;
                    }
                }
            }
        });
    }

    // --------- Envío manual por stdin ----------
    use tokio::io::AsyncBufReadExt; // (ya importado arriba)
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    println!("Escribe un mensaje para enviar al ESP32 (exit para salir):");
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().eq_ignore_ascii_case("exit") {
            println!("👋 Saliendo...");
            break;
        }
        if let Err(e) = socket.send_to(line.as_bytes(), &remote_addr).await {
            error!("❌ Error enviando: {e}");
        } else {
            println!("📤 Sent to {} -> {}", remote_addr, line);
        }
    }

    // --------- Servidor HTTP ----------
    {
        let http_ctx = ws_ctx.clone();
        let _http_server = tokio::spawn(async move {
            info!("🌐 Iniciando servidor HTTP en http://0.0.0.0:3000");
            match start_http_server(http_ctx).await {
                Ok(_) => info!("✅ Servidor HTTP detenido"),
                Err(e) => error!("❌ Error en servidor HTTP: {e}"),
            }
        });
    }

    Ok(())
}