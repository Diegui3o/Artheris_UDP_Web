use std::net::SocketAddr;
use std::sync::Arc;
use std::collections::HashSet;
use std::env;

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{self, json};
use serde_json::Value;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{broadcast, RwLock};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tracing::{error, info, warn};

use crate::config::function::{
    set_led_all, set_led_one, set_mode,
    set_motors_state, set_motors_all_speed, set_motors_many_speed
};
use super::questdb::OptionalDb;
use anyhow::Context;

// Helper function to get current system mode
async fn get_current_mode() -> Result<i32> {
    // Default mode
    Ok(0)
}

// Helper function to extract request_id from JSON
fn get_request_id(value: &Value) -> Option<String> {
    value.get("request_id").and_then(|v| v.as_str()).map(|s| s.to_string())
}

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
    leds: Option<LedMany>, // many
    command: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<Payload>,
    mode: Option<i32>,
    command: Option<String>,
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
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            set: HashSet::new(),
            last_updated: Utc::now(),
        }
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
    _questdb: &OptionalDb,
    _flight_id: &Arc<RwLock<Option<String>>>,
    _last_config: &Arc<RwLock<Option<Value>>>,
    _available_fields: &Arc<RwLock<AvailableFieldIndex>>,
) -> Result<()> {
    let msg: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            error!("❌ Error parsing JSON: {}", e);
            return Ok(());
        }
    };

    // Handle mode change
    if let Some(mode) = msg.get("mode") {
        let mode_str = if let Some(s) = mode.as_str() {
            s.to_string()
        } else if let Some(n) = mode.as_i64() {
            n.to_string()
        } else {
            error!("❌ Invalid mode format");
            return Ok(());
        };
        
        let request_id = get_request_id(&msg);
        set_mode(
            &mode_str,
            esp32_socket.clone(),
            *remote_addr,
            tx,
            request_id.as_deref(),
        ).await;
        return Ok(());
    }

    // Handle motor commands
    if let Some(cmd) = msg.get("command").and_then(|c| c.as_str()) {
        match cmd {
            "MOTORS_ON" => {
                set_motors_state(true, esp32_socket.clone(), *remote_addr, tx, None).await;
            }
            "MOTORS_OFF" => {
                set_motors_state(false, esp32_socket.clone(), *remote_addr, tx, None).await;
            }
            _ => {
                warn!("Unknown command: {}", cmd);
            }
        }
        return Ok(());
    }

    // Handle motor speed commands
    if let Some(motors) = msg.get("motors").and_then(|m| m.as_object()) {
        if let (Some(ids), Some(speed)) = (
            motors.get("ids").and_then(|v| v.as_array()),
            motors.get("speed").and_then(|v| v.as_u64()),
        ) {
            let ids: Vec<u32> = ids.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect();
            if !ids.is_empty() {
                set_motors_many_speed(
                    &ids,
                    speed as u32,
                    esp32_socket.clone(),
                    *remote_addr,
                    tx,
                    None,
                ).await;
            }
        } else if let Some(speed) = motors.get("speed").and_then(|v| v.as_u64()) {
            set_motors_all_speed(
                speed as u32,
                esp32_socket.clone(),
                *remote_addr,
                tx,
                None,
            ).await;
        }
        return Ok(());
    }

    // Handle LED commands
    if let Some(led) = msg.get("led") {
        if let Some(led_obj) = led.as_object() {
            if let (Some(id), Some(state)) = (led_obj.get("id"), led_obj.get("state")) {
                if let (Some(id), Some(state)) = (id.as_u64(), state.as_bool()) {
                    set_led_one(
                        id as u32,
                        state,
                        esp32_socket.clone(),
                        *remote_addr,
                        tx,
                        None,
                    ).await;
                }
            }
        } else if let Some(state) = led.as_bool() {
            set_led_all(
                state,
                esp32_socket.clone(),
                *remote_addr,
                tx,
                None,
            ).await;
        }
        return Ok(());
    }

    // Forward the message as is if not handled above
    if let Some(socket) = esp32_socket {
        if let Err(e) = socket.send_to(text.as_bytes(), remote_addr).await {
            error!("❌ Error forwarding message to ESP32: {}", e);
        }
    }

    // Parse the incoming message as JSON
    let msg: Value = serde_json::from_str(text).context("Failed to parse WebSocket message as JSON")?;
    
    // Handle different types of messages
    if let Some(msg_type) = msg.get("type").and_then(|t| t.as_str()) {
        match msg_type {
            "get_mode" => {
                // Return current mode to the client
                if let Ok(mode) = get_current_mode().await {
                    let response = json!({ "type": "mode", "mode": mode });
                    tx.send(response.to_string())?;
                }
            }
            "get_snapshot" => {
                // Helper function to get system snapshot
                async fn get_system_snapshot() -> Result<Value> {
                    // Return a basic snapshot
                    Ok(json!({ 
                        "status": "ok",
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                        "version": env!("CARGO_PKG_VERSION", "unknown")
                    }))
                }

                // Return current system snapshot to the client
                if let Ok(snapshot) = get_system_snapshot().await {
                    let response = json!({ "type": "snapshot", "data": snapshot });
                    tx.send(response.to_string())?;
                }
            }
            _ => {
                info!("Received unhandled message type: {}", msg_type);
            }
        }
    }
    
    // Broadcast the message to all connected clients
    tx.send(text.to_string())?;
    
    Ok(())
}

pub async fn start_ws_server(ctx: WsContext) -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:9001").await?;
    info!("🔌 WebSocket server listening on ws://0.0.0.0:9001");

    while let Ok((stream, _addr)) = listener.accept().await {
        let ctx = ctx.clone();
        
        tokio::spawn(async move {

            // Aceptar la conexión WebSocket
            let ws_stream = match accept_hdr_async(stream, |_: &Request, mut response: Response| {
                response.headers_mut().append("Access-Control-Allow-Origin", "*".parse().unwrap());
                response.headers_mut().append("Access-Control-Allow-Methods", "GET, POST, OPTIONS".parse().unwrap());
                response.headers_mut().append("Access-Control-Allow-Headers", "*".parse().unwrap());
                Ok(response)
            }).await {
                Ok(ws) => ws,
                Err(e) => {
                    error!("❌ Error during WebSocket handshake: {}", e);
                    return;
                }
            };
            
            let (mut ws_sender, mut ws_receiver) = ws_stream.split();
            let mut rx = ctx.tx.subscribe();
            
            // Tarea para enviar mensajes al cliente
            let send_task = async move {
                while let Ok(msg) = rx.recv().await {
                    if ws_sender.send(Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
            };
            
            // Tarea para recibir mensajes del cliente
            let recv_task = async {
                while let Some(Ok(msg)) = ws_receiver.next().await {
                    if let Message::Text(text) = msg {
                        if let Err(e) = handle_ws_message(
                            &text,
                            &ctx.tx,
                            &ctx.esp32_socket,
                            &ctx.remote_addr,
                            &ctx.questdb,
                            &ctx.flight_id,
                            &ctx.last_config,
                            &ctx.available_fields,
                        ).await {
                            error!("❌ Error handling WebSocket message: {}", e);
                        }
                    }
                }
            };
            
            // Ejecutar ambas tareas concurrentemente
            tokio::select! {
                _ = send_task => {}
                _ = recv_task => {}
            }
            
        });
    }
    
    Ok(())
}
