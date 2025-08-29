use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::AsyncBufReadExt;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, RwLock};

use tracing::{error, info, warn};
use tracing_appender::rolling;
use tracing_subscriber::{fmt, EnvFilter};
use tracing_subscriber::prelude::*;

use serde_json::Value;
use std::collections::HashSet;

mod config;
mod ws_server;

use crate::ws_server::questdb::{OptionalDb, QuestDb, QuestDbConfig};
use crate::ws_server::{start_http_server, start_ws_server, WsContext};

// 👉 Debes exportar esto desde ws_server::server
// pub struct AvailableFieldIndex { pub set: HashSet<String>, pub last_updated: DateTime<Utc>, ... }
use crate::ws_server::server::AvailableFieldIndex;

// =======================
// Logging
// =======================
fn init_logging() -> anyhow::Result<()> {
    let file_appender = rolling::daily("./logs", "artheris.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(std::io::stdout)
                .with_target(false)
                .with_level(true),
        )
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_target(false)
                .with_level(true),
        )
        .with(EnvFilter::from_default_env().add_directive("debug".parse()?))
        .try_init()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    info!("🚀 Iniciando Artheris UDP/Web");
    Ok(())
}

fn discover_numeric_keys(v: &serde_json::Value) -> Vec<String> {
    use serde_json::Value;

    fn try_parse_json_str(s: &str) -> Option<Value> {
        let t = s.trim_start();
        if t.starts_with('{') || t.starts_with('[') {
            serde_json::from_str::<Value>(t).ok()
        } else { None }
    }

    fn walk(prefix: &str, node: &Value, out: &mut Vec<String>) {
        match node {
            Value::Number(_) => {
                if !prefix.is_empty() { out.push(prefix.to_string()); }
            }
            Value::Object(map) => {
                for (k, v) in map {
                    let key = if prefix.is_empty() { k.clone() } else { format!("{}.{}", prefix, k) };
                    match v {
                        Value::Number(_) => out.push(key),
                        Value::String(s) => {
                            if let Some(parsed) = try_parse_json_str(s) {
                                walk(&key, &parsed, out);
                            }
                        }
                        Value::Array(arr) => {
                            // no indexamos [0],[1] para no ensuciar nombres; caminamos elementos
                            for it in arr { walk(&key, it, out); }
                        }
                        _ => walk(&key, v, out),
                    }
                }
            }
            Value::Array(arr) => {
                for it in arr { walk(prefix, it, out); }
            }
            Value::String(s) => {
                if let Some(parsed) = try_parse_json_str(s) {
                    walk(prefix, &parsed, out);
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    if let Some(payload) = v.get("payload") {
        walk("", payload, &mut out);
    } else {
        walk("", v, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn extract_numeric_record_and_time(
    v: &serde_json::Value,
    allowlist: Option<&HashSet<String>>,
    time_field_override: Option<&str>,
    mode_field_override: Option<&str>,
) -> Option<(
    serde_json::Map<String, serde_json::Value>,
    Option<String>,
    Option<String>,
)> {
    use serde_json::{Map, Value};

    let obj = if let Some(obj) = v.as_object() {
        if let Some(payload) = obj.get("payload").and_then(|p| p.as_object()) {
            payload
        } else {
            obj
        }
    } else {
        return None;
    };

    let candidate_time_names = [
        time_field_override.unwrap_or(""),
        "time", "timestamp", "ts", "Time", "Timestamp", "TS",
    ];
    let candidate_mode_names = [
        mode_field_override.unwrap_or(""),
        "mode", "modo", "modoActual", "Mode", "MODE",
    ];

    let mut fields = Map::new();
    let mut ts_field: Option<String> = None;
    let mut mode_val: Option<String> = None;

    for (k, val) in obj {
        if ts_field.is_none()
            && candidate_time_names
                .iter()
                .any(|n| !n.is_empty() && *n == k)
        {
            ts_field = Some(k.clone());
            continue;
        }

        if mode_val.is_none()
            && candidate_mode_names
                .iter()
                .any(|n| !n.is_empty() && *n == k)
        {
            let s = if let Some(s) = val.as_str() {
                s.to_string()
            } else if let Some(n) = val.as_i64() {
                n.to_string()
            } else if let Some(f) = val.as_f64() {
                format!("{}", f)
            } else {
                continue;
            };
            mode_val = Some(s);
            continue;
        }

        if let Some(allow) = allowlist {
            if !allow.contains(k) {
                continue;
            }
        }

        if val.is_number() {
            fields.insert(k.clone(), val.clone());
        }
    }

    if fields.is_empty() {
        return None;
    }
    Some((fields, ts_field, mode_val))
}

// =======================
// Main
// =======================
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(e) = init_logging() {
        eprintln!("❌ No se pudo inicializar el logging: {e}");
        return Err(e);
    }

    // ---------- QuestDB ----------
    let questdb_config = QuestDbConfig {
        host: std::env::var("QUESTDB_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        port: std::env::var("QUESTDB_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8812),
        user: std::env::var("QUESTDB_USER").unwrap_or_else(|_| "admin".into()),
        password: std::env::var("QUESTDB_PASSWORD").unwrap_or_else(|_| "quest".into()),
        database: std::env::var("QUESTDB_DB").unwrap_or_else(|_| "qdb".into()),
        table_name: Some("flight_telemetry".to_string()),
        time_col: Some("timestamp".to_string()),
    };
    info!(
        "🔧 Configuración de QuestDB: host={} port={}",
        questdb_config.host, questdb_config.port
    );

    let qdb = {
        let db = OptionalDb::new(questdb_config.clone());
        match QuestDb::connect(questdb_config.clone()).await {
            Ok(_) => {
                info!("✅ Conectado a QuestDB");
                db
            }
            Err(e) => {
                warn!(
                    "⚠️  No se pudo conectar a QuestDB al inicio: {e}. Se intentará bajo demanda."
                );
                db
            }
        }
    };

    // ---------- Estado compartido ----------
    let current_flight_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
    let last_config: Arc<RwLock<Option<serde_json::Value>>> = Arc::new(RwLock::new(None));

    // Índice de campos disponibles (para /api/telemetry/fields)
    let available_fields = Arc::new(RwLock::new(AvailableFieldIndex::new()));

    // Canal broadcast WS
    let (tx, _) = broadcast::channel::<String>(100);

    // ---------- UDP ----------
    const LOCAL_PORT: u16 = 8889;
    const REMOTE_IP: &str = "192.168.1.50";
    const REMOTE_PORT: u16 = 8888;

    let local_addr = format!("0.0.0.0:{LOCAL_PORT}");
    let remote_addr: SocketAddr = format!("{REMOTE_IP}:{REMOTE_PORT}")
        .parse()
        .expect("SocketAddr");

    let socket = Arc::new(UdpSocket::bind(&local_addr).await?);
    println!("✅ UDP listening on {local_addr}");

    // ---------- WsContext ----------
    let ws_ctx = WsContext {
        tx: tx.clone(),
        esp32_socket: Some(socket.clone()),
        remote_addr,
        questdb: qdb.clone(),
        flight_id: current_flight_id.clone(),
        last_config: last_config.clone(),
        available_fields: available_fields.clone(), // 👈 NUEVO
    };

    // ---------- WS server ----------
    let _ws_server = tokio::spawn({
        let ctx = ws_ctx.clone();
        async move {
            info!("🔌 Iniciando servidor WebSocket en ws://0.0.0.0:9001");
            start_ws_server(ctx).await;
            info!("✅ Servidor WebSocket detenido");
        }
    });

    // ---------- HTTP server ----------
    let _http_server = tokio::spawn({
        let ctx = ws_ctx.clone();
        async move {
            info!("🌍 Iniciando servidor HTTP en http://0.0.0.0:3000");
            if let Err(e) = start_http_server(ctx).await {
                error!("❌ Error en el servidor HTTP: {e}");
            }
        }
    });

    // ---------- Tarea UDP RX ----------
    {
        let socket_recv = Arc::clone(&socket);
        let tx_udp = tx.clone();
        let qdb_writer = qdb.clone();
        let flight_state = current_flight_id.clone();
        let fields_index = available_fields.clone();
        let cfg_ref = last_config.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                match socket_recv.recv_from(&mut buf).await {
                    Ok((len, _src)) => {
                        if let Ok(text) = std::str::from_utf8(&buf[..len]) {
                            // Normaliza a {"type":"telemetry","payload":...}
                            let (to_ws, to_store) =
                                match serde_json::from_str::<serde_json::Value>(text) {
                                    Ok(v) => match v.get("type").and_then(|t| t.as_str()) {
                                        Some("ack") | Some("telemetry") => (v.to_string(), Some(v)),
                                        _ => {
                                            let wrapped = serde_json::json!({
                                                "type": "telemetry",
                                                "payload": v
                                            });
                                            (wrapped.to_string(), Some(wrapped))
                                        }
                                    },
                                    Err(_) => {
                                        // texto plano -> payload string
                                        let wrapped = serde_json::json!({
                                            "type": "telemetry",
                                            "payload": text
                                        });
                                        (wrapped.to_string(), Some(wrapped))
                                    }
                                };

                            // WS broadcast
                            let _ = tx_udp.send(to_ws);

                            if let Some(flog) = to_store {
                                // 👉 Descubrir/actualizar catálogo de campos
                                tracing::debug!("📡 Raw telemetry data: {}", serde_json::to_string_pretty(&flog).unwrap_or_else(|_| "[invalid json]".to_string()));
                                let found_keys = discover_numeric_keys(&flog);
                                if !found_keys.is_empty() {
                                    let mut idx = available_fields.write().await; // o fields_index según tu variable
                                    let changed = idx.merge_keys(found_keys);
                                    if changed {
                                        tracing::info!("🆕 Índice actualizado: {} campos (last_updated={})",
                                            idx.set.len(), idx.last_updated.to_rfc3339());
                                    }
                                }
                                if !found_keys.is_empty() {
                                    let mut idx = available_fields.write().await;
                                    idx.merge_keys(found_keys);
                                }
                                if !found_keys.is_empty() {
                                    let mut idx = fields_index.write().await;
                                    let changed = idx.merge_keys(found_keys);
                                    if changed {
                                        tracing::debug!(
                                            "📚 Campos descubiertos/actualizados: {} (last_updated={})",
                                            idx.set.len(),
                                            idx.last_updated.to_rfc3339()
                                        );
                                    }
                                }

                                // Ingest a QuestDB SOLO si hay flight_id activo
                                let fid_opt = { flight_state.read().await.clone() };
                                if let Some(ref fid) = fid_opt {
                                    // Guarda crudo en flight_logs
                                    if let Err(e) = qdb_writer
                                        .insert_flight_log(fid, &flog.to_string())
                                        .await
                                    {
                                        error!("❌ [flight_logs] insert_flight_log falló: {e}");
                                    }

                                    // Lee config para construir allowlist + overrides
                                    let cfg_snapshot = { cfg_ref.read().await.clone() };

                                    let (allowlist, time_field_override, mode_field_override) =
                                        if let Some(cfg) = cfg_snapshot.as_ref() {
                                            let allow: HashSet<String> = cfg
                                                .get("selectedFields")
                                                .and_then(|a| a.as_array())
                                                .map(|arr| {
                                                    arr.iter()
                                                        .filter_map(|v| {
                                                            v.as_str().map(|s| s.to_string())
                                                        })
                                                        .collect()
                                                })
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

                                    // Extrae campos numéricos filtrados y manda a ILP
                                    match extract_numeric_record_and_time(
                                        &flog,
                                        allowlist.as_ref(),
                                        time_field_override.as_deref(),
                                        mode_field_override.as_deref(),
                                    ) {
                                        Some((record_obj, ts_field_opt, mode_val_opt)) => {
                                            let rec_json =
                                                serde_json::Value::Object(record_obj);
                                            let mode_opt_str = mode_val_opt.as_deref();

                                            match qdb_writer
                                                .ingest_telemetry_batch(
                                                    fid,
                                                    "1", // schema_version
                                                    mode_opt_str,
                                                    std::slice::from_ref(&rec_json),
                                                    ts_field_opt.as_deref(),
                                                )
                                                .await
                                            {
                                                Ok(n) => tracing::info!(
                                                    "✅ [flight_telemetry] ILP ok: inserted={} flight_id={}",
                                                    n,
                                                    fid
                                                ),
                                                Err(e) => {
                                                    tracing::error!(
                                                        "❌ [flight_telemetry] ILP ingest falló: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                        None => {
                                            tracing::debug!("ℹ️ No hubo campos numéricos tras filtrar/soportar timestamp; flog={}", flog);
                                        }
                                    }
                                } // if flight_id
                            } // if to_store
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

    // ---------- Envío manual por stdin ----------
    {
        let stdin = tokio::io::BufReader::new(tokio::io::stdin());
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
    }

    Ok(())
}
