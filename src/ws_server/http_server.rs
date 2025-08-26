use crate::ws_server::WsContext;
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::post,
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
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

#[derive(Debug, Default)]
struct AppState {
    current_flight_id: RwLock<Option<String>>,
    current_config: RwLock<Option<LoggerConfig>>,
}

pub async fn start_http_server(_ctx: WsContext) -> anyhow::Result<()> {
    let app_state = Arc::new(AppState::default());
    
    let app = Router::new()
        .route("/api/logger/config", post(apply_config))
        .route("/api/recordings/start", post(start_recording))
        .route("/api/recordings/stop", post(stop_recording))
        .with_state(app_state);

    let addr = "0.0.0.0:3000";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("🚀 Servidor HTTP iniciado en {}", addr);
    
    axum::serve(listener, app).await?;
    
    Ok(())
}

async fn apply_config(
    State(state): State<Arc<AppState>>,
    Json(config): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let config: LoggerConfig = match serde_json::from_value(config) {
        Ok(c) => c,
        Err(e) => {
            return Err((
                StatusCode::BAD_REQUEST, 
                format!("Invalid config format: {}", e)
            ))
        }
    };
    
    *state.current_config.write().await = Some(config);
    Ok(Json(serde_json::json!({
        "status": "config_applied"
    })))
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
    
    if let Some(id) = flight_id {
        Ok(Json(serde_json::json!({
            "status": "recording_stopped",
            "flight_id": id
        })))
    } else {
        Err((StatusCode::BAD_REQUEST, "No active recording".to_string()))
    }
}