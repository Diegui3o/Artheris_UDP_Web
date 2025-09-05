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
use tokio::sync::mpsc;
use bytes::Bytes;
use tracing_subscriber::prelude::*;
use tokio::time::{Duration, Instant};
use socket2::{Socket, Domain, Type, Protocol};

use crate::ws_server::{
    start_ws_server, WsContext, AvailableFieldIndex, OptionalDb, start_http_server,
    questdb::{QuestDb, QuestDbConfig}
};

use std::sync::atomic::{AtomicU64, Ordering};
static FIELDS_SAMPLER: AtomicU64 = AtomicU64::new(0);
const FIELDS_SAMPLE_EVERY: u64 = 50; // 1 de cada 50
static UDP_RX_PKTS:     AtomicU64 = AtomicU64::new(0);
static DISP_DROPS:      AtomicU64 = AtomicU64::new(0);
static WORKER_DROPS:    AtomicU64 = AtomicU64::new(0);
static ILP_LINES_SENT:  AtomicU64 = AtomicU64::new(0);

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
        .with(EnvFilter::from_default_env().add_directive("info".parse()?))
        .try_init()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    info!("🚀 Iniciando Artheris UDP/Web");
    Ok(())
}

// Escribe el batch en una sola llamada ILP y limpia el vector
async fn flush_batch(
    qdb_writer: &OptionalDb,
    flight_state: &Arc<RwLock<Option<String>>>,
    batch: &mut Vec<serde_json::Value>,
) -> anyhow::Result<()> {
    let fid_opt = { flight_state.read().await.clone() };
    if let Some(fid) = fid_opt {
        if !batch.is_empty() {
            qdb_writer
                .ingest_telemetry_batch(&fid, "1", None, &*batch, Some("timestamp"))
                .await
                .map_err(|e| anyhow::anyhow!("Failed to ingest batch: {}", e))?;
            //tracing::info!("🟢 ILP batch ok: {} rows (flight_id={})", batch.len(), fid);
        }
    }
    batch.clear();
    Ok(())
}

fn extract_numeric_record_and_time(
    v: &serde_json::Value,
    allowlist: Option<&std::collections::HashSet<String>>,
    time_field_override: Option<&str>,
    mode_field_override: Option<&str>,
) -> Option<(
    serde_json::Map<String, serde_json::Value>,
    Option<String>,
    Option<String>,
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
    let std_sock = make_udp(&local_addr, 128 * 1024 * 1024)?; // 128 MB
    let socket: Arc<UdpSocket> = Arc::new(UdpSocket::from_std(std_sock)?);
    println!("✅ UDP listening on {}", local_addr);
    let available_fields = Arc::new(RwLock::new(AvailableFieldIndex::default()));

    fn make_udp(local: &str, rcvbuf_bytes: usize) -> std::io::Result<std::net::UdpSocket> {
        let addr: SocketAddr = local.parse().expect("bad addr");
        let domain = if addr.is_ipv4() { Domain::IPV4 } else { Domain::IPV6 };
        let sock = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))?;
    
        // Permite rebind rápido si reinicias
        sock.set_reuse_address(true).ok();
        #[cfg(target_family = "unix")]
        sock.set_reuse_port(true).ok();
    
        // **Aumenta buffer de recepción**
        sock.set_recv_buffer_size(rcvbuf_bytes)?;
        // IMPORTANTE: en algunos SO el kernel “clampa” el valor; luego lo leemos para loguear
        let _ = sock.bind(&addr.into())?;
    
        // Non-blocking para Tokio
        sock.set_nonblocking(true)?;
    
        // Log del valor real aplicado
        if let Ok(applied) = sock.recv_buffer_size() {
            println!("🧰 SO_RCVBUF solicitado={} MB, aplicado≈{} MB",
                rcvbuf_bytes / (1024*1024), applied / (1024*1024));
        }
    
        Ok(sock.into())
    }
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
            //info!("✅ Servidor WebSocket detenido");
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
    
    // Metrics collection task
    tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_secs(5));
        let mut last_udp = 0u64;
        let mut last_drp = 0u64;
        let mut last_ilp = 0u64;
        loop {
            t.tick().await;
            let udp = UDP_RX_PKTS.load(Ordering::Relaxed);
            let drp = DISP_DROPS.load(Ordering::Relaxed);
            let ilp = ILP_LINES_SENT.load(Ordering::Relaxed);
    
            let d_udp = udp - last_udp;
            let d_drp = drp - last_drp;
            let d_ilp = ilp - last_ilp;
    
            last_udp = udp;
            last_drp = drp;
            last_ilp = ilp;
        }
    });
// === PIPELINE UDP: producer + dispatcher + workers ===
const RX_QUEUE: usize = 40_000;
const WORKERS: usize  = 4;
const PER_WORKER_Q: usize = RX_QUEUE / WORKERS;

// 1) Cola cruda global (Bytes) del productor al dispatcher
let (tx_raw, mut rx_raw) = mpsc::channel::<Bytes>(RX_QUEUE);

// 2) Producer: SOLO IO; NO parsea JSON
{
    let socket_recv = socket.clone();
    let tx_raw = tx_raw.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65_536];
        loop {
            match socket_recv.recv_from(&mut buf).await {
                Ok((len, _)) => {
                    UDP_RX_PKTS.fetch_add(1, Ordering::Relaxed);
                    let slice = Bytes::copy_from_slice(&buf[..len]); // copia mínima
                    if tx_raw.try_send(slice).is_err() {
                    }
                }
                Err(e) => {
                    tracing::error!("UDP recv error: {e}");
                    break;
                }
            }
        }
    });
}

// 3) Crea N colas de worker y lanza workers
let mut worker_senders = Vec::with_capacity(WORKERS);
for _ in 0..WORKERS {
    let (txw, mut rxw) = mpsc::channel::<Bytes>(PER_WORKER_Q);
    worker_senders.push(txw);

    // Capturas compartidas
    let tx_ws        = tx.clone();
    let qdb_writer   = questdb.clone();
    let flight_state = current_flight_id.clone();
    let last_config  = last_config.clone();
    let fields_index = available_fields.clone();

    tokio::spawn(async move {
        const BATCH_MAX: usize = 1500;
        const BATCH_MS:  u64   = 250;

        let mut batch: Vec<serde_json::Value> = Vec::with_capacity(BATCH_MAX);
        let mut ticker     = tokio::time::interval(Duration::from_millis(BATCH_MS));
        let mut last_flush = Instant::now();

        loop {
            tokio::select! {
                Some(bytes) = rxw.recv() => {
                    // parse rápido (best-effort)
                    let parsed = match serde_json::from_slice::<serde_json::Value>(&bytes) {
                        Ok(v) => v,
                        Err(_) => match std::str::from_utf8(&bytes) {
                            Ok(s) => serde_json::json!({ "type":"telemetry", "payload": s }),
                            Err(_) => continue, // binario desconocido
                        }
                    };

                    // normaliza para WS/DB
                    let normalized = match parsed.get("type").and_then(|t| t.as_str()) {
                        Some("ack") | Some("telemetry") => parsed,
                        _ => serde_json::json!({ "type":"telemetry", "payload": parsed }),
                    };

                    // broadcast WS (best-effort)
                    let _ = tx_ws.send(normalized.to_string());

                    // Sample fields occasionally to reduce lock contention
                    let ticket = FIELDS_SAMPLER.fetch_add(1, Ordering::Relaxed);
                    if ticket % FIELDS_SAMPLE_EVERY == 0 {
                        let ks = discover_numeric_keys(&normalized);
                        if !ks.is_empty() {
                            let mut idx = fields_index.write().await;
                            idx.merge_keys(ks);
                        }
                    }

                    // allowlist + overrides desde /api/config
                    let (allowlist, t_override, m_override) = {
                        let snap = { last_config.read().await.clone() };
                        if let Some(cfg) = snap.as_ref() {
                            use std::collections::HashSet;
                            let allow: HashSet<String> = cfg.get("selectedFields")
                                .and_then(|a| a.as_array())
                                .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                                .unwrap_or_default();
                            let t = cfg.pointer("/metadata/timeField").and_then(|v| v.as_str()).map(str::to_string);
                            let m = cfg.pointer("/metadata/modeField").and_then(|v| v.as_str()).map(str::to_string);
                            (Some(allow), t, m)
                        } else { (None, None, None) }
                    };

                    // extrae 1..N registros numéricos y acumula en batch
                    let mut push_numeric = |one: &serde_json::Value| {
                        if let Some((obj, _ts, _mode)) = extract_numeric_record_and_time(
                            one, allowlist.as_ref(), t_override.as_deref(), m_override.as_deref()
                        ) {
                            batch.push(serde_json::Value::Object(obj));
                        }
                    };
                    if let Some(arr) = normalized.get("payload").and_then(|p| p.as_array()) {
                        for it in arr { push_numeric(it); }
                    } else {
                        push_numeric(&normalized);
                    }

                    // política de flush
                    if batch.len() >= BATCH_MAX || last_flush.elapsed() > Duration::from_millis(BATCH_MS) {
                        let _ = flush_batch(&qdb_writer, &flight_state, &mut batch).await;
                        last_flush = Instant::now();
                    }
                }
                _ = ticker.tick() => {
                    if !batch.is_empty() {
                        let _ = flush_batch(&qdb_writer, &flight_state, &mut batch).await;
                        last_flush = Instant::now();
                    }
                }
            }
        }
    });
}

// 4) Dispatcher: reparte round-robin desde rx_raw → workers; dropea si cola de worker llena
{
    let senders = worker_senders;
    tokio::spawn(async move {
        let mut i = 0usize;
        while let Some(b) = rx_raw.recv().await {
            let txw = &senders[i % senders.len()];
            if txw.try_send(b).is_err() {
                DISP_DROPS.fetch_add(1, Ordering::Relaxed);
            }
            i = i.wrapping_add(1);
        }
    });
}
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