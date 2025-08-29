use std::net::SocketAddr;
use std::sync::Arc;
use std::collections::HashSet;

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{broadcast, RwLock};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::config::function::{set_led_all, set_led_many, set_led_one, set_motors_state, set_mode};
use super::questdb::OptionalDb;

/// Estructuras para decodificar comandos de alto nivel
#[derive(Debug, Deserialize)]
struct LedOne {
    id: u32,
    state: bool,
}

#[derive(Debug, Deserialize)]
struct LedMany {
    ids: Vec<u32>,
    state: bool,
}

#[derive(Debug, Deserialize)]
struct Payload {
    mode: Option<i32>,
    motors: Option<bool>,
    led: Option<Value>,   // bool | {id,state}
    leds: Option<LedMany> // many
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<Payload>,
    mode: Option<i32>,       // formato directo
    command: Option<String>, // legacy
}

#[derive(Debug, Clone)]
pub struct AvailableFieldIndex {
    pub set: HashSet<String>,
    pub last_updated: DateTime<Utc>,
}

impl Default for AvailableFieldIndex {
    fn default() -> Self {
        Self { 
            set: HashSet::new(), 
            last_updated: Utc::now() 
        }
    }
}

impl AvailableFieldIndex {
    pub fn new() -> Self { 
        Self::default() 
    }
    
    pub fn merge_keys<I: IntoIterator<Item = String>>(&mut self, iter: I) -> bool {
        let mut changed = false;
        let mut new_fields = Vec::new();
        
        for k in iter {
            if self.set.insert(k.clone()) { 
                changed = true;
                new_fields.push(k);
            }
        }
        
        if changed {
            self.last_updated = Utc::now();
            tracing::info!(
                "🆕 Added {} new fields. Total fields now: {}",
                new_fields.len(),
                self.set.len()
            );
            if !new_fields.is_empty() {
                tracing::debug!("   New fields: {:?}", new_fields);
            }
        } else {
            //tracing::debug!("ℹ️  No new fields to add. Total fields: {}", self.set.len());
        }
        
        changed
    }
}

/// Comando específico que estabas usando en el WS → QuestDB
/// { "type": "data", "flight_id": "X", "payload": "<json string o texto>" }
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Command {
    Data { 
        flight_id: String, 
        payload: String 
    },
}

/// Contexto compartido para WS/HTTP
#[derive(Clone, Debug)]
pub struct WsContext {
    pub tx: broadcast::Sender<String>,
    pub esp32_socket: Option<Arc<UdpSocket>>,
    pub remote_addr: SocketAddr,
    pub questdb: OptionalDb,
    pub flight_id: Arc<RwLock<Option<String>>>,
    pub last_config: Arc<RwLock<Option<Value>>>,
    pub available_fields: Arc<RwLock<AvailableFieldIndex>>,
}

pub async fn start_ws_server(ctx: WsContext) -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:9001").await?;
    info!("🌐 WebSocket server escuchando en ws://0.0.0.0:9001");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let mut rx = ctx.tx.subscribe();
        let ctx_clone = ctx.clone();

        tokio::spawn(async move {
            let ws = match accept_async(stream).await {
                Ok(ws) => ws,
                Err(e) => {
                    error!("❌ Error aceptando WS: {}", e);
                    return;
                }
            };

            let (ws_sender, mut ws_receiver) = ws.split();
            let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));

            // Task 1: broadcast -> cliente
            let mut rx_task = {
                let ws_sender = Arc::clone(&ws_sender);
                tokio::spawn(async move {
                    while let Ok(text) = rx.recv().await {
                        if ws_sender.lock().await.send(Message::Text(text)).await.is_err() {
                            break;
                        }
                    }
                })
            };

            // Task 2: cliente -> router/UDP/DB
            let mut recv_task = {
                let ws_sender = Arc::clone(&ws_sender);
                tokio::spawn(async move {
                    while let Some(msg) = ws_receiver.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                debug!("📨 WS: {text}");

                                // Reenvía a ESP32 si está conectado
                                if let Some(sock) = &ctx_clone.esp32_socket {
                                    if let Err(e) = sock.send_to(text.as_bytes(), ctx_clone.remote_addr).await {
                                        error!("❌ Error enviando a ESP32: {e}");
                                    }
                                }

                                // Persistencia si es Command::Data
                                if let Ok(Command::Data { flight_id, payload }) =
                                    serde_json::from_str::<Command>(&text)
                                {
                                    if let Err(e) = ctx_clone.questdb.insert_flight_log(&flight_id, &payload).await {
                                        warn!("⚠️  {}", e);
                                    }
                                    // Reenvía a todos los clientes WebSocket
                                    if let Err(e) = ctx_clone.tx.send(text.clone()) {
                                        error!("❌ Error enviando broadcast: {e}");
                                    }
                                } else {
                                    // Si no es Command::Data, igual lo publicamos a clientes
                                    let _ = ctx_clone.tx.send(text);
                                }
                            }
                            Ok(Message::Ping(p)) => {
                                let _ = ws_sender.lock().await.send(Message::Pong(p)).await;
                            }
                            Ok(Message::Pong(_)) => {}
                            Ok(Message::Binary(_)) => {}
                            Ok(Message::Close(_)) => break,
                            Ok(Message::Frame(_)) => {}
                            Err(e) => {
                                error!("❌ Error recibiendo WS: {}", e);
                                break;
                            }
                        }
                    }
                })
            };

            // Espera a que una de las tasks termine
            tokio::select! {
                _ = &mut rx_task => recv_task.abort(),
                _ = &mut recv_task => rx_task.abort(),
            }
        });
    }
}

async fn handle_incoming(
    text: &str,
    esp32_socket: Option<Arc<UdpSocket>>,
    remote_addr: SocketAddr,
    ws_tx: &broadcast::Sender<String>,
) -> anyhow::Result<()> {
    let root: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            // No es JSON → re-publica y listo
            let _ = ws_tx.send(text.to_string());
            return Ok(());
        }
    };

    let kind = root.get("type").and_then(|v| v.as_str());

    // request_id top-level o dentro de payload
    let req_id_top = root.get("request_id").and_then(|v| v.as_str());
    let req_id_in_payload = root
        .get("payload")
        .and_then(|p| p.get("request_id"))
        .and_then(|v| v.as_str());
    let req_id = req_id_top.or(req_id_in_payload);

    // Comando puede estar en root.payload o root.payload.payload
    let payload_top = root.get("payload");
    let payload_inner = payload_top.and_then(|p| p.get("payload"));
    let command_node = payload_inner.or(payload_top);

    let env = serde_json::from_value::<Envelope>(root.clone()).ok();

    // A) type: "command"
    if matches!(kind, Some("command")) {
        if let Some(cmd) = command_node {
            // leds many
            if let Some(leds_node) = cmd.get("leds") {
                if let Ok(many) = serde_json::from_value::<LedMany>(leds_node.clone()) {
                    set_led_many(&many.ids, many.state, esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                    return Ok(());
                }
            }
            // led all / one
            if let Some(led_node) = cmd.get("led") {
                if let Some(all) = led_node.as_bool() {
                    set_led_all(all, esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                    return Ok(());
                }
                if let Ok(one) = serde_json::from_value::<LedOne>(led_node.clone()) {
                    set_led_one(one.id, one.state, esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                    return Ok(());
                }
            }
            // mode
            if let Some(m) = cmd.get("mode").and_then(|v| v.as_i64()) {
                set_mode(&m.to_string(), esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                return Ok(());
            }
            // motors
            if let Some(motors) = cmd.get("motors").and_then(|v| v.as_bool()) {
                set_motors_state(motors, esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                return Ok(());
            }
            // passthrough prudente
            if let Some(sock) = &esp32_socket {
                sock.send_to(text.as_bytes(), remote_addr).await?;
            }
            return Ok(());
        }
    }

    // B) Formatos alternativos (Envelope)
    if let Some(env) = env {
        if matches!(env.kind.as_deref(), Some("command")) {
            if let Some(p) = env.payload {
                if let Some(m) = p.mode {
                    set_mode(&m.to_string(), esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                    return Ok(());
                }
                if let Some(motors) = p.motors {
                    set_motors_state(motors, esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                    return Ok(());
                }
                if let Some(many) = p.leds {
                    set_led_many(&many.ids, many.state, esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                    return Ok(());
                }
                if let Some(led_val) = p.led {
                    if let Some(all) = led_val.as_bool() {
                        set_led_all(all, esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                        return Ok(());
                    }
                    if let Ok(one) = serde_json::from_value::<LedOne>(led_val) {
                        set_led_one(one.id, one.state, esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
                        return Ok(());
                    }
                }
            }
        }

        if let Some(m) = env.mode {
            set_mode(&m.to_string(), esp32_socket.clone(), remote_addr, ws_tx, req_id).await;
            return Ok(());
        }

        if let Some(cmd) = env.command.as_deref() {
            match cmd {
                "ON_LED"     => set_led_all(true,  esp32_socket.clone(), remote_addr, ws_tx, req_id).await,
                "OFF_LED"    => set_led_all(false, esp32_socket.clone(), remote_addr, ws_tx, req_id).await,
                "ON_MOTORS"  => set_motors_state(true,  esp32_socket.clone(), remote_addr, ws_tx, req_id).await,
                "OFF_MOTORS" => set_motors_state(false, esp32_socket.clone(), remote_addr, ws_tx, req_id).await,
                _ => {
                    if let Some(sock) = &esp32_socket {
                        sock.send_to(text.as_bytes(), remote_addr).await?;
                    }
                }
            }
            return Ok(());
        }
    }

    // JSON válido pero no reconocido → passthrough
    if let Some(sock) = &esp32_socket {
        sock.send_to(text.as_bytes(), remote_addr).await?;
    }
    Ok(())
}
