use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::broadcast;

/// Mapa de alias -> número
fn mode_str_to_num(s: &str) -> Option<u8> {
    let s = s.trim().to_ascii_lowercase();
    match s.as_str() {
        "pilot" | "piloto" => Some(0),
        "idle" | "espera" => Some(1),
        "manual" => Some(2),
        // si te mandan "0", "1" o "2" como string
        _ => s.parse::<u8>().ok().filter(|n| [0u8, 1, 2].contains(n)),
    }
}

/// Envía modo como **número** si es posible (mejor para el ESP)
pub async fn set_mode(
    mode: &str, // acepta "pilot", "manual", "idle|espera", o "0|1|2"
    esp32_socket: Option<Arc<UdpSocket>>,
    remote_addr: SocketAddr,
    ws_tx: &broadcast::Sender<String>,
    request_id: Option<&str>,
) {
    // 1) Normaliza a número si podemos
    let json_payload = if let Some(n) = mode_str_to_num(mode) {
        json!({"type":"command","payload":{"mode": n}})
    } else {
        // fallback: envía string tal cual
        json!({"type":"command","payload":{"mode": mode}})
    };

    let txt = json_payload.to_string();

    // 2) Enviar por UDP
    let mut ok = true;
    if let Some(socket) = esp32_socket {
        if let Err(e) = socket.send_to(txt.as_bytes(), remote_addr).await {
            eprintln!("❌ Error enviando modo al ESP32: {}", e);
            ok = false;
        }
    } else {
        ok = false;
    }

    // 3) ACK opcional
    if let Some(rid) = request_id {
        let ack = if ok {
            json!({"type":"ack","request_id": rid, "ok": true})
        } else {
            json!({"type":"ack","request_id": rid, "ok": false, "info":"udp_send_failed_or_missing_socket"})
        };
        let _ = ws_tx.send(ack.to_string());
    }

    // 4) Broadcast para la UI en el formato exacto que espera el frontend
    let mode_num = mode_str_to_num(mode).unwrap_or_else(|| mode.parse::<u8>().unwrap_or(0));

    // Primero enviamos un mensaje de tipo 'mode_update' que es el que maneja el frontend
    let update_msg = json!({
        "type": "mode_update",
        "mode": mode_num,
        "modoActual": mode_num,
        "value": mode_num,
        "status": "ok"
    });
    let _ = ws_tx.send(update_msg.to_string());

    // Enviamos un mensaje de tipo 'modo' para compatibilidad
    let modo_msg = json!({
        "type": "modo",
        "value": mode_num,
        "mode": mode_num,
        "modoActual": mode_num
    });
    let _ = ws_tx.send(modo_msg.to_string());

    // Enviamos un mensaje de estado completo
    let status_msg = json!({
        "type": "status",
        "mode": mode_num,
        "modoActual": mode_num,
        "status": "ok"
    });
    let _ = ws_tx.send(status_msg.to_string());

    // Enviamos un mensaje directo con modoActual en la raíz para asegurar compatibilidad
    let direct_msg = json!({
        "modoActual": mode_num,
        "mode": mode_num,
        "type": "status_update"
    });
    let _ = ws_tx.send(direct_msg.to_string());

    println!("📤 Enviando comando de MODO al ESP32: {}", mode);
}

pub async fn set_motor_one_speed(
    id: u32,
    us: u32,
    esp32_socket: Option<Arc<UdpSocket>>,
    remote_addr: SocketAddr,
    ws_tx: &broadcast::Sender<String>,
    request_id: Option<&str>,
) {
    let payload = json!({
        "type":"command",
        "payload": { "motor": { "id": id, "speed": us } }
    });
    let txt = payload.to_string();

    let mut ok = true;
    if let Some(sock) = esp32_socket {
        if let Err(e) = sock.send_to(txt.as_bytes(), remote_addr).await {
            eprintln!("❌ Error enviando MOTOR ONE SPEED: {e}");
            ok = false;
        }
    } else {
        ok = false;
    }

    if let Some(rid) = request_id {
        let _ = ws_tx.send(
            json!({
                "type":"ack", "request_id": rid, "ok": ok
            })
            .to_string(),
        );
    }
    let _ = ws_tx.send(
        json!({
            "type":"motor","target":"one","id": id,"speed": us
        })
        .to_string(),
    );
}

pub async fn set_motors_many_speed(
    ids: &[u32],
    us: u32,
    esp32_socket: Option<Arc<UdpSocket>>,
    remote_addr: SocketAddr,
    ws_tx: &broadcast::Sender<String>,
    request_id: Option<&str>,
) {
    let payload = json!({
        "type":"command",
        "payload": { "motors": { "ids": ids, "speed": us } }
    });
    let txt = payload.to_string();

    let mut ok = true;
    if let Some(sock) = esp32_socket {
        if let Err(e) = sock.send_to(txt.as_bytes(), remote_addr).await {
            eprintln!("❌ Error enviando MOTORS MANY SPEED: {e}");
            ok = false;
        }
    } else {
        ok = false;
    }

    if let Some(rid) = request_id {
        let _ = ws_tx.send(
            json!({
                "type":"ack", "request_id": rid, "ok": ok
            })
            .to_string(),
        );
    }
    if ok {
        for &id in ids {
            let _ = ws_tx.send(
                json!({
                    "type":"motor","target":"one","id": id,"speed": us
                })
                .to_string(),
            );
        }
    }
}

pub async fn set_motors_all_speed(
    us: u32,
    esp32_socket: Option<Arc<UdpSocket>>,
    remote_addr: SocketAddr,
    ws_tx: &broadcast::Sender<String>,
    request_id: Option<&str>,
) {
    let payload = json!({
        "type":"command",
        "payload": { "motors": { "speed": us } }
    });
    let txt = payload.to_string();

    let mut ok = true;
    if let Some(sock) = esp32_socket {
        if let Err(e) = sock.send_to(txt.as_bytes(), remote_addr).await {
            eprintln!("❌ Error enviando MOTORS ALL SPEED: {e}");
            ok = false;
        }
    } else {
        ok = false;
    }

    if let Some(rid) = request_id {
        let _ = ws_tx.send(
            json!({
                "type":"ack", "request_id": rid, "ok": ok
            })
            .to_string(),
        );
    }
    let _ = ws_tx.send(
        json!({
            "type":"motors","target":"all","speed": us
        })
        .to_string(),
    );
}

pub async fn set_led_all(
    on: bool,
    esp32_socket: Option<Arc<UdpSocket>>,
    remote_addr: SocketAddr,
    ws_tx: &broadcast::Sender<String>,
    request_id: Option<&str>,
) {
    let payload = json!({
        "type": "command",
        "payload": { "led": on }
    });
    let txt = payload.to_string();

    let mut ok = true;
    if let Some(sock) = esp32_socket {
        if let Err(e) = sock.send_to(txt.as_bytes(), remote_addr).await {
            eprintln!("❌ Error enviando LED ALL al ESP32: {}", e);
            ok = false;
        }
    } else {
        ok = false;
    }
    if let Some(rid) = request_id {
        let ack = if ok {
            json!({"type":"ack","request_id": rid, "ok": true})
        } else {
            json!({"type":"ack","request_id": rid, "ok": false, "info":"udp_send_failed_or_missing_socket"})
        };
        let _ = ws_tx.send(ack.to_string());
    }
    let _ = ws_tx.send(json!({"type":"led","target":"all","value": on}).to_string());
}

/// Un LED específico
pub async fn set_led_one(
    id: u32,
    on: bool,
    esp32_socket: Option<Arc<UdpSocket>>,
    remote_addr: SocketAddr,
    ws_tx: &broadcast::Sender<String>,
    request_id: Option<&str>,
) {
    let payload = json!({
        "type": "command",
        "payload": { "led": { "id": id, "state": on } }
    });
    let txt = payload.to_string();

    let mut ok = true;
    if let Some(sock) = esp32_socket {
        if let Err(e) = sock.send_to(txt.as_bytes(), remote_addr).await {
            eprintln!("❌ Error enviando LED ONE al ESP32: {}", e);
            ok = false;
        }
    } else {
        ok = false;
    }
    if let Some(rid) = request_id {
        let ack = if ok {
            json!({"type":"ack","request_id": rid, "ok": true})
        } else {
            json!({"type":"ack","request_id": rid, "ok": false, "info":"udp_send_failed_or_missing_socket"})
        };
        let _ = ws_tx.send(ack.to_string());
    }
    let _ = ws_tx.send(json!({"type":"led","target":"one","id": id,"value": on}).to_string());
}

/// Varios LEDs a la vez
pub async fn set_led_many(
    ids: &[u32],
    on: bool,
    esp32_socket: Option<Arc<UdpSocket>>,
    remote_addr: SocketAddr,
    ws_tx: &broadcast::Sender<String>,
    request_id: Option<&str>,
) {
    let payload = json!({
        "type": "command",
        "payload": { "leds": { "ids": ids, "state": on } }
    });
    let txt = payload.to_string();

    let mut ok = true;
    if let Some(sock) = esp32_socket {
        if let Err(e) = sock.send_to(txt.as_bytes(), remote_addr).await {
            eprintln!("❌ Error enviando LED MANY al ESP32: {}", e);
            ok = false;
        }
    } else {
        ok = false;
    }
    if let Some(rid) = request_id {
        let ack = if ok {
            json!({"type":"ack","request_id": rid, "ok": true})
        } else {
            json!({"type":"ack","request_id": rid, "ok": false, "info":"udp_send_failed_or_missing_socket"})
        };
        let _ = ws_tx.send(ack.to_string());
    }
    if ok {
        for &id in ids {
            let _ =
                ws_tx.send(json!({"type":"led","target":"one","id": id,"value": on}).to_string());
        }
    }
}

/// Enciende o apaga los motores y notifica
pub async fn set_motors_state(
    motors_on: bool,
    esp32_socket: Option<Arc<UdpSocket>>,
    remote_addr: SocketAddr,
    ws_tx: &broadcast::Sender<String>,
    request_id: Option<&str>, // 👈 nuevo
) {
    let command = format!(
        r#"{{"type":"command","payload":{{"motors":{}}}}}"#,
        motors_on
    );

    let mut ok = true;
    if let Some(socket) = esp32_socket {
        if let Err(e) = socket.send_to(command.as_bytes(), remote_addr).await {
            eprintln!("❌ Error enviando motores al ESP32: {}", e);
            ok = false;
        }
    } else {
        ok = false;
    }

    // ACK
    if let Some(rid) = request_id {
        let ack = if ok {
            json!({"type":"ack","request_id": rid, "ok": true})
        } else {
            json!({"type":"ack","request_id": rid, "ok": false, "info":"udp_send_failed_or_missing_socket"})
        };
        let _ = ws_tx.send(ack.to_string());
    }
    let _ = ws_tx.send(json!({"type":"motors","value": motors_on}).to_string());

    println!(
        "📤 Enviando comando de MOTORES al ESP32: {}",
        if motors_on { "ON" } else { "OFF" }
    );
}
