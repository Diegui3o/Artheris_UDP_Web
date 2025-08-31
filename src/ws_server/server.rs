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
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tracing::{error, info};

use crate::config::function::{set_led_all, set_led_many, set_led_one, set_motors_state, set_mode};
use super::questdb::OptionalDb;
use anyhow::Context;

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
                //tracing::debug!("   New fields: {:?}", new_fields);
            }
        } else {
            //tracing::debug!("ℹ️  No new fields to add. Total fields: {}", self.set.len());
        }
        
        changed
    }
}

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

async fn handle_ws_message(
    text: &str,
    tx: &broadcast::Sender<String>,
    esp32_socket: &Option<Arc<UdpSocket>>,
    remote_addr: &SocketAddr,
    questdb: &OptionalDb,
    flight_id: &Arc<RwLock<Option<String>>>,
    last_config: &Arc<RwLock<Option<Value>>>,
    available_fields: &Arc<RwLock<AvailableFieldIndex>>,
) -> Result<()> {
    // Parse the incoming message as JSON
    let msg: Value = serde_json::from_str(text).context("Failed to parse WebSocket message as JSON")?;
    
    // Handle different types of messages
    if let Some(cmd_type) = msg.get("type").and_then(|t| t.as_str()) {
        match cmd_type {
            "command" => {
                // Forward the command to ESP32 if socket is available
                if let Some(socket) = esp32_socket {
                    if let Some(cmd) = msg.get("command").and_then(|c| c.as_str()) {
                        socket.send_to(cmd.as_bytes(), remote_addr).await?;
                        info!("Forwarded command to ESP32: {}", cmd);
                    }
                }
            }
            "config" => {
                // Update last known configuration
                let mut config = last_config.write().await;
                *config = Some(msg.clone());
                info!("Updated configuration");
            }
            _ => {
                info!("Received unhandled message type: {}", cmd_type);
            }
        }
    }
    
    // Broadcast the message to all connected clients
    tx.send(text.to_string())?;
    
    Ok(())
}

pub async fn start_ws_server(ctx: WsContext) -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:9001").await?;
    info!("🌐 WebSocket server escuchando en ws://0.0.0.0:9001");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!("🔌 New connection from: {}", addr);
                let ctx_clone = ctx.clone();

                tokio::spawn(async move {
                    // Configure CORS and other headers for WebSocket handshake
                    let callback = |req: &Request, mut response: Response| {
                        info!("🔍 WebSocket handshake for {} at {}", req.uri(), addr);
                        let headers = response.headers_mut();
                        headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
                        headers.insert("Access-Control-Allow-Methods", "GET".parse().unwrap());
                        headers.insert("Access-Control-Allow-Headers", "content-type".parse().unwrap());
                        Ok(response)
                    };

                    match accept_hdr_async(stream, callback).await {
                        Ok(ws_stream) => {
                            info!("✅ WebSocket connection established with {}", addr);
                            let (ws_sender, mut ws_receiver) = ws_stream.split();
                            let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));
                            let mut rx = ctx_clone.tx.subscribe();

                            // Create a channel for sending messages to the WebSocket
                            let (ws_tx, mut ws_rx) = tokio::sync::mpsc::channel::<Message>(32);
                            
                            // Task 1: Handle incoming messages from the client
                            let tx_clone = ctx_clone.tx.clone();
                            let esp32_socket_clone = ctx_clone.esp32_socket.clone();
                            let remote_addr_clone = ctx_clone.remote_addr;
                            let questdb_clone = ctx_clone.questdb.clone();
                            let flight_id_clone = ctx_clone.flight_id.clone();
                            let last_config_clone = ctx_clone.last_config.clone();
                            let available_fields_clone = ctx_clone.available_fields.clone();

                            let mut ws_task = tokio::spawn(async move {
                                while let Some(Ok(msg)) = ws_receiver.next().await {
                                    match msg {
                                        Message::Text(text) => {
                                            if let Err(e) = handle_ws_message(
                                                &text,
                                                &tx_clone,
                                                &esp32_socket_clone,
                                                &remote_addr_clone,
                                                &questdb_clone,
                                                &flight_id_clone,
                                                &last_config_clone,
                                                &available_fields_clone,
                                            ).await {
                                                error!("Error handling WebSocket message: {}", e);
                                                break;
                                            }
                                            // Broadcast to other clients if needed
                                            if let Err(e) = tx_clone.send(text) {
                                                error!("Failed to broadcast message: {}", e);
                                                break;
                                            }
                                        }
                                        Message::Ping(p) => {
                                            if ws_tx.send(Message::Pong(p)).await.is_err() {
                                                break;
                                            }
                                        }
                                        Message::Close(_) => break,
                                        _ => {}
                                    }
                                }
                            });

                            // Task 2: Send messages to the WebSocket
                            let mut rx_task = {
                                let ws_sender = ws_sender.clone();
                                tokio::spawn(async move {
                                    while let Some(msg) = ws_rx.recv().await {
                                        if ws_sender.lock().await.send(msg).await.is_err() {
                                            error!("❌ Failed to send WebSocket message to {}", addr);
                                            break;
                                        }
                                    }
                                    info!("📤 Message sender task ended for {}", addr);
                                })
                            };

                            // Task 3: Broadcast messages to this client
                            let mut broadcast_task = {
                                let ws_sender = ws_sender.clone();
                                tokio::spawn(async move {
                                    while let Ok(text) = rx.recv().await {
                                        if ws_sender.lock().await.send(Message::Text(text)).await.is_err() {
                                            error!("❌ Failed to broadcast message to {}", addr);
                                            break;
                                        }
                                    }
                                    info!("📤 Broadcast task ended for {}", addr);
                                })
                            };

                            // Wait for either task to complete
                            tokio::select! {
                                _ = &mut rx_task => {
                                    info!("📭 Broadcast task completed for {}", addr);
                                    ws_task.abort();
                                }
                                _ = &mut ws_task => {
                                    info!("📭 WebSocket task completed for {}", addr);
                                    rx_task.abort();
                                }
                            };
                        }
                        Err(e) => {
                            error!("❌ WebSocket error with {}: {}", addr, e);
                        }
                    }
                });
            }
            Err(e) => {
                error!("❌ Error accepting connection: {}", e);
            }
        }
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
