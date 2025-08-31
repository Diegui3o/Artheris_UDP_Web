use std::net::SocketAddr;
use std::sync::Arc;
use tokio::{
    net::UdpSocket,
    sync::{broadcast, RwLock},
    io::BufReader
};
use tracing::{info, error};
use tracing_subscriber::{EnvFilter, fmt};
use tracing_appender::rolling;

mod config;
mod ws_server;

use tracing_subscriber::prelude::*;

use crate::ws_server::{
    start_ws_server, WsContext, AvailableFieldIndex, OptionalDb, start_http_server,
    questdb::{QuestDb, QuestDbConfig}
};

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
    allowlist: Option<&std::collections::HashSet<String>>,
    time_field_override: Option<&str>,
    mode_field_override: Option<&str>,
) -> Option<(
    serde_json::Map<String, serde_json::Value>, // fields numéricos filtrados
    Option<String>,                              // nombre del campo timestamp (si existe)
    Option<String>,                              // modo (si existe)
)> {
    use serde_json::Map;
    // 1) localizar objeto
    let obj = if let Some(obj) = v.as_object() {
        if let Some(payload) = obj.get("payload").and_then(|p| p.as_object()) {
            payload
        } else {
            obj
        }
    } else {
        return None;
    };

    // 2) determinar nombres especiales
    let candidate_time_names = [
        time_field_override.unwrap_or(""),
        "time","timestamp","ts","Time","Timestamp","TS",
    ];
    let candidate_mode_names = [
        mode_field_override.unwrap_or(""),
        "mode","modo","modoActual","Mode","MODE",
    ];

    // 3) recorrer y filtrar
    let mut fields = Map::new();
    let mut ts_field: Option<String> = None;
    let mut mode_val: Option<String> = None;

    for (k, val) in obj {
        // detectar timestamp
        if ts_field.is_none() && candidate_time_names.iter().any(|n| !n.is_empty() && *n == k) {
            ts_field = Some(k.clone());
            continue;
        }
        // detectar modo (lo usaremos como tag)
        if mode_val.is_none() && candidate_mode_names.iter().any(|n| !n.is_empty() && *n == k) {
            // conviértelo a string, sea número o texto
            let s = if let Some(s) = val.as_str() {
                s.to_string()
            } else if let Some(n) = val.as_i64() {
                n.to_string()
            } else if let Some(f) = val.as_f64() {
                // evita notación científica rara
                format!("{}", f)
            } else {
                continue;
            };
            mode_val = Some(s);
            continue;
        }

        // aplicar allowlist (si existe)
        if let Some(allow) = allowlist {
            if !allow.contains(k) {
                continue;
            }
        }

        // solo numéricos
        if val.is_number() {
            fields.insert(k.clone(), val.clone());
        }
    }

    if fields.is_empty() {
        return None;
    }
    Some((fields, ts_field, mode_val))
}

fn discover_numeric_keys(v: &serde_json::Value) -> Vec<String> {
    use serde_json::Value;

    fn walk(prefix: &str, val: &Value, out: &mut Vec<String>) {
        match val {
            Value::Number(_) => {
                if !prefix.is_empty() {
                    out.push(prefix.to_string());
                }
            }
            Value::Object(map) => {
                for (k, v) in map {
                    let p = if prefix.is_empty() { k.clone() } else { format!("{}.{}", prefix, k) };
                    walk(&p, v, out);
                }
            }
            Value::Array(arr) => {
                // Recorre elementos pero NO agregues índice al nombre, así deduplica
                for v in arr { walk(prefix, v, out); }
            }
            Value::String(s) => {
                if (s.starts_with('{') || s.starts_with('[')) && serde_json::from_str::<Value>(s).is_ok() {
                    if let Ok(inner) = serde_json::from_str::<Value>(s) {
                        walk(prefix, &inner, out);
                    }
                }
            }
            _ => {}
        }
    }

    let root = v.as_object()
        .and_then(|o| o.get("payload"))
        .unwrap_or(v);

    let mut out = Vec::new();
    walk("", root, &mut out);
    out.sort();
    out.dedup();
    out
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

    // Initialize QuestDB connection
    let questdb = match QuestDb::connect(questdb_config.clone()).await {
        Ok(db) => {
            info!("Connected to QuestDB");
            OptionalDb {
                inner: Arc::new(tokio::sync::Mutex::new(Some(db))),
                config: questdb_config,
            }
        }
        Err(e) => {
            error!("Failed to connect to QuestDB: {}", e);
            OptionalDb {
                inner: Arc::new(tokio::sync::Mutex::new(None)),
                config: questdb_config,
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
    let socket: Arc<UdpSocket> = Arc::new(UdpSocket::bind(local_addr.clone()).await?);
    println!("✅ UDP listening on {}", local_addr);
    let available_fields = Arc::new(RwLock::new(AvailableFieldIndex::default()));

    let ws_ctx = WsContext {
        tx: tx.clone(),
        esp32_socket: Some(socket.clone()),
        remote_addr,
        questdb: questdb.clone(),
        flight_id: current_flight_id.clone(),
        last_config: last_config.clone(),
        available_fields: available_fields.clone(),
    };
    // WS server
    let _ws_server = tokio::spawn({
        let ctx = ws_ctx.clone();
        async move {
            info!("🔌 Iniciando servidor WebSocket en ws://0.0.0.0:9001");
            let _ = start_ws_server(ctx).await;
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

    let _udp_handler = {
        let socket_recv: Arc<UdpSocket> = Arc::clone(&socket);
        let tx_udp = tx.clone();
        let qdb_writer = questdb.clone();
        let flight_state = current_flight_id.clone();
        let last_config = last_config.clone();
        let fields_index = available_fields.clone();

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
                                    // 2) Guarda crudo en flight_logs
                                    if let Err(e) = qdb_writer.insert_flight_log(&fid, &flog.to_string()).await {
                                        error!("Error guardando log de vuelo: {}", e);
                                    }
                                    // 3) Get current configuration for filtering
                                    let cfg_snapshot = { last_config.read().await.clone() };

                                    // Build allowlist from selectedFields and get field overrides
                                    use std::collections::HashSet;
                                    let (allowlist, time_field_override, mode_field_override) = if let Some(cfg) = cfg_snapshot.as_ref() {
                                        let allow: HashSet<String> = cfg
                                            .get("selectedFields")
                                            .and_then(|a| a.as_array())
                                            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                                            .unwrap_or_else(HashSet::new);

                                        let time_field = cfg
                                            .pointer("/metadata/timeField")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string());

                                        let mode_field = cfg
                                            .pointer("/metadata/modeField")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string());

                                        (Some(allow), time_field, mode_field)
                                    } else {
                                        (None, None, None)
                                    };

                                    // 4) Extract numeric fields with filtering and field overrides
                                    match extract_numeric_record_and_time(
                                        &flog,
                                        allowlist.as_ref(),
                                        time_field_override.as_deref(),
                                        mode_field_override.as_deref(),
                                    ) {
                                        Some((record_obj, _ts_field_opt, mode_val_opt)) => {
                                            let rec_json = serde_json::Value::Object(record_obj);
                                            let _mode_opt_str = mode_val_opt.as_deref();

                                            let records = vec![rec_json];
                                            match qdb_writer.ingest_telemetry_batch(
                                                &fid,
                                                "1",
                                                None,
                                                &records,
                                                Some("timestamp"),
                                            ).await {
                                                Ok(n) => {
                                                    // n = líneas ILP enviadas (debería ser 1)
                                                    tracing::info!("✅ [flight_telemetry] ILP ok: inserted={} flight_id={}", n, fid);
                                                }
                                                Err(e) => {
                                                    // Cuando esto falle, verás el porqué aquí
                                                    tracing::error!("❌ [flight_telemetry] ILP ingest falló: {}", e);
                                                }
                                            }
                                        }
                                        None => {
                                            // No se pudo extraer un objeto numérico (tal vez el payload no es el esperado)
                                            tracing::debug!("ℹ️ No hubo campos numéricos tras filtrar/soportar timestamp; flog={:?}", flog);
                                        }
                                    }
                                } else {
                                    //tracing::debug!("ℹ️ Sin flight_id activo: ignorando insert a flight_telemetry");
                                }
                                
                                // 📚 Actualiza el índice SIEMPRE con lo que llega por UDP
                                let keys = discover_numeric_keys(&flog);
                                if !keys.is_empty() {
                                    let mut idx = fields_index.write().await;
                                    if idx.merge_keys(keys) {
                                        tracing::info!(
                                            "📚 Índice actualizado: total={} (last_updated={})",
                                            idx.set.len(),
                                            idx.last_updated.to_rfc3339()
                                        );
                                    }
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
        })
    };

    // --------- Envío manual por stdin ----------
    use tokio::io::AsyncBufReadExt;
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

    Ok(())
}