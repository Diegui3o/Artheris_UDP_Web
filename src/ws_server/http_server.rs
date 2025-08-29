use crate::ws_server::WsContext;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::info;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoggerConfig {
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    #[serde(rename = "selectedFields")]
    selected_fields: Vec<String>,
    retention: RetentionConfig,
    triggers: TriggerConfig,
    metadata: Option<MetadataConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum RetentionConfig {
    Infinite { mode: String },
    Ttl { mode: String, seconds: u64 },
}

#[derive(Debug, Serialize, Deserialize)]
struct TriggerConfig {
    #[serde(rename = "startWhen")]
    start_when: StartCondition,
    #[serde(rename = "stopWhen")]
    stop_when: Option<StopCondition>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StartCondition {
    key: String,
    between: [f64; 2],
}

#[derive(Debug, Serialize, Deserialize)]
struct StopCondition {
    key: String,
    #[serde(rename = "outsideForSeconds")]
    outside_for_seconds: u64,
    range: [f64; 2],
}

#[derive(Debug, Serialize, Deserialize)]
struct MetadataConfig {
    mass: Option<f64>,
    #[serde(rename = "armLength")]
    arm_length: Option<f64>,
}

#[derive(Debug)]
struct AppState {
    ws_ctx: Arc<Mutex<WsContext>>,
    current_flight_id: RwLock<Option<String>>,
    current_config: RwLock<Option<LoggerConfig>>,
}

impl AppState {
    fn new(ws_ctx: WsContext) -> Self {
        Self {
            ws_ctx: Arc::new(Mutex::new(ws_ctx)),
            current_flight_id: RwLock::new(None),
            current_config: RwLock::new(None),
        }
    }
}

pub async fn start_http_server(ctx: WsContext) -> anyhow::Result<()> {
    let app_state = Arc::new(AppState::new(ctx));
    
    // Create CORS layer
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    
    let app = Router::new()
        .route("/api/logger/config", post(apply_config))
        .route("/api/recordings/start", post(start_recording))
        .route("/api/recordings/stop", post(stop_recording))
        .route("/api/telemetry/fields", get(get_available_fields_handler))
        .with_state(app_state.clone())
        .layer(cors)
        .layer(TraceLayer::new_for_http());
    let addr = "0.0.0.0:3000";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("🚀 Servidor HTTP iniciado en {}", addr);
    
    axum::serve(listener, app).await?;
    
    Ok(())
}

async fn get_available_fields_handler(
    State(state): State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    let ctx = state.ws_ctx.lock().await;
    let idx = ctx.available_fields.read().await;
    
    let mut fields: Vec<String> = idx.set.iter().cloned().collect();
    fields.sort();
    
    #[derive(Serialize)]
    struct FieldsResponse {
        fields: Vec<String>,
        last_updated: String,
    }
    
    axum::Json(FieldsResponse {
        fields,
        last_updated: idx.last_updated.to_rfc3339(),
    })
}

pub async fn apply_config(
    State(ctx): State<Arc<Mutex<WsContext>>>,
    Json(cfg): Json<LoggerConfig>,
) -> impl IntoResponse {
    let ctx = ctx.lock().await;

    {
        let mut last = ctx.last_config.write().await;
        *last = Some(cfg.rest.clone());
    }

    // 🌱 Sembrar índice con selectedFields
    if let Some(arr) = cfg.rest.get("selectedFields").and_then(|v| v.as_array()) {
        let keys: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
        if !keys.is_empty() {
            let mut idx = ctx.available_fields.write().await;
            if idx.merge_keys(keys) {
                tracing::info!("🌱 Índice sembrado desde apply_config (size={})", idx.set.len());
            }
        }
    }

    if let Err(e) = ctx.questdb.insert_logger_config(&cfg.rest.to_string()).await {
        eprintln!("⚠️  {e}");
    }

    Json(ApiOk { status: "ok".into() })
}

async fn start_recording(
    State(state): State<Arc<AppState>>,
    Json(config): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let config: LoggerConfig = match serde_json::from_value(config.clone()) {
        Ok(c) => c,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST, 
                format!("Invalid config format: {}\nConfig: {}", e, config)
            ))
        }
    };
    
    let flight_id = Uuid::new_v4().to_string();
    *state.current_flight_id.write().await = Some(flight_id.clone());
    *state.current_config.write().await = Some(config);
    
    // Notify WebSocket clients about the new recording
    let ws_tx = &state.ws_ctx.lock().await.tx;
    let _ = ws_tx.send(serde_json::json!({
        "type": "recording_started",
        "flight_id": flight_id
    }).to_string());
    
    let response = serde_json::json!({
        "status": "recording_started",
        "flight_id": flight_id
    });
    
    Ok(Json(response))
}

async fn stop_recording(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let flight_id = state.current_flight_id.write().await.take();
    
    if let Some(id) = &flight_id {
        // Notify WebSocket clients that recording has stopped
        let ws_tx = &state.ws_ctx.lock().await.tx;
        let _ = ws_tx.send(serde_json::json!({
            "type": "recording_stopped",
            "flight_id": id
        }).to_string());
        
        Ok(Json(serde_json::json!({
            "status": "recording_stopped",
            "flight_id": id
        })))
    } else {
        Err((StatusCode::BAD_REQUEST, "No active recording".to_string()))
    }
}