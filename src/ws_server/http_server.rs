use crate::ws_server::WsContext;
use crate::ws_server::stats::IngestStats;
use serde_json::Value;
use tracing::warn;
use axum::{
    extract::{Path, Query, State},
    http::{Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use crate::config::handlers::{
    get_flight_metrics, 
    get_flight_metrics_full, 
    get_error_comparison, 
    get_flight_spectrum,
    get_flight_uncertainty,
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{self, error, info};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use uuid::Uuid;
use std::sync::atomic::Ordering;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggerConfig {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "selectedFields")]
    pub selected_fields: Vec<String>,
    pub retention: RetentionConfig,
    pub triggers: TriggerConfig,
    pub metadata: Option<MetadataConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RetentionConfig {
    Infinite { mode: String },
    Ttl { mode: String, seconds: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerConfig {
    #[serde(rename = "startWhen")]
    pub start_when: StartCondition,
    #[serde(rename = "stopWhen")]
    pub stop_when: Option<StopCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartCondition {
    pub key: String,
    pub greater_than: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopCondition {
    pub key: String,
    pub less_than: f64,
    #[serde(rename = "afterSeconds")]
    pub after_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataConfig {
    pub mass: Option<f64>,
    #[serde(rename = "armLength")]
    pub arm_length: Option<f64>,
}

#[derive(Debug)]
pub struct AppState {
    pub ws_ctx: Arc<Mutex<WsContext>>,
    pub current_flight_id: RwLock<Option<String>>,
    pub current_config: RwLock<Option<LoggerConfig>>,
    pub stats: IngestStats,
}

impl AppState {
    fn new(ws_ctx: WsContext) -> Self {
        Self {
            ws_ctx: Arc::new(Mutex::new(ws_ctx)),
            current_flight_id: RwLock::new(None),
            current_config: RwLock::new(None),
            stats: IngestStats::default(),
        }
    }
}

pub async fn start_http_server(ctx: WsContext) -> anyhow::Result<()> {
    let app_state = Arc::new(AppState::new(ctx));
    let app = routes(app_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    tracing::info!("🚀 Servidor HTTP iniciado en 0.0.0.0:3000");

    axum::serve(listener, app).await?;
    Ok(())
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        // Rutas que tu frontend usa
        .route("/api/telemetry/fields", get(get_available_fields_handler))
        .route("/api/config", post(apply_config))
        .route("/api/start", post(start_recording))
        .route("/api/stop", post(stop_recording))
        .route("/api/stats", get(get_stats))

        // Data API
        .route("/api/flights", get(list_flights))
        .route("/api/flights/:id/series", get(get_flight_series))
        .route("/api/flights/:fid/summary", get(get_flight_summary))
        .route("/api/flights/:fid/debug", get(get_flight_debug))
        .route("/api/ingest", post(ingest_points))

        // Aliases antiguos
        .route("/api/available-fields", get(get_available_fields_handler))
        .route("/api/start-recording", post(start_recording))
        .route("/api/stop-recording", post(stop_recording))
        .route("/api/flights/:id/metrics", get(get_flight_metrics))
        .route("/api/flights/:id/metrics-full", get(get_flight_metrics_full))
        .route("/api/flights/:id/error-comparison", get(get_error_comparison))
        .route("/api/flights/:id/spectrum", get(get_flight_spectrum))
        .route("/api/flights/:id/uncertainty", get(get_flight_uncertainty))
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers(Any)
        )
        .layer(TraceLayer::new_for_http())
}

#[derive(Serialize)]
struct FlightDebugInfo {
    flight_id: String,
    start_ts: String,
    end_ts: String,
    point_count: usize,
    fields: HashMap<String, String>,
    first_point: Option<Value>,
    last_point: Option<Value>,
}

async fn get_flight_debug(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<FlightDebugInfo>, ApiError> {
    // Get the questdb client and keep the lock for the duration of the function
    let ctx = state.ws_ctx.lock().await;
    
    // Get all points for the flight (just first and last point)
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 2)
        .await
        .map_err(|e| {
            eprintln!("❌ get_flight_debug: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;

    if points.is_empty() {
        return Err(ApiError::NotFound(format!("Flight {} not found", fid)));
    }

    // Analyze fields in the points
    let mut fields = HashMap::new();
    if let Some(first) = points.first() {
        if let Some(payload) = first.payload.get("payload").and_then(|v| v.as_object()) {
            for (k, v) in payload {
                fields.insert(k.clone(), format!("{:?}", v));
            }
        }
    }

    let first_point = points.first().map(|p| p.payload.clone());
    let last_point = points.last().map(|p| p.payload.clone());

    Ok(Json(FlightDebugInfo {
        flight_id: fid,
        start_ts: points.first().unwrap().ts.to_rfc3339(),
        end_ts: points.last().unwrap().ts.to_rfc3339(),
        point_count: points.len(),
        fields,
        first_point,
        last_point,
    }))
}

async fn get_available_fields_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let ctx = state.ws_ctx.lock().await;
    let idx = ctx.available_fields.read().await;

    let mut fields: Vec<String> = idx.set.iter().cloned().collect();
    fields.sort();

    #[derive(Serialize)]
    struct FieldsResponse {
        fields: Vec<String>,
        last_updated: String,
    }

    Json(FieldsResponse {
        fields,
        last_updated: idx.last_updated.to_rfc3339(),
    })
}

#[axum::debug_handler]
pub async fn apply_config(
    State(state): State<Arc<AppState>>,
    Json(cfg): Json<LoggerConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Guarda en AppState
    {
        let mut cur = state.current_config.write().await;
        *cur = Some(cfg.clone());
    }

    // Guarda en WsContext.last_config y en DB
    let cfg_val = serde_json::to_value(&cfg).unwrap_or(json!({}));
    let cfg_str = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".into());

    {
        let ctx = state.ws_ctx.lock().await;
        *ctx.last_config.write().await = Some(cfg_val);
        // OptionalDb -> llama directo
        if let Err(e) = ctx.questdb.insert_logger_config(&cfg_str).await {
            eprintln!("⚠️  Failed to save config to database: {e}");
        }
    }

    Ok(Json(json!({ "status": "ok" })))
}

#[axum::debug_handler]
pub async fn start_recording(
    State(state): State<Arc<AppState>>,
    Json(cfg): Json<LoggerConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    info!("🔴 start_recording: INICIANDO");
    
    let flight_id = Uuid::new_v4().to_string();
    info!("🔴 flight_id generado: {}", flight_id);

    // ⭐ Crear metadatos
    let metadata = crate::models::experiment_metadata::ExperimentMetadata {
        experiment_id: format!("EXP_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S")),
        flight_id: flight_id.clone(),
        start_time: chrono::Utc::now(),
        end_time: None,
        duration_seconds: None,
        sampling_rate_hz: 25,
        esp32_loop_hz: 1000,
        filter_type: "kalman".to_string(),
        kalman_gains: None,
        experiment_type: crate::models::experiment_metadata::ExperimentType::Manual,
        description: None,
        location: None,
        notes: None,
    };
    
    info!("🔴 Metadatos creados: experiment_id={}", metadata.experiment_id);

    // Guarda en AppState
    {
        let mut cur = state.current_config.write().await;
        *cur = Some(cfg.clone());
        let mut fid = state.current_flight_id.write().await;
        *fid = Some(flight_id.clone());
    }
    info!("🔴 AppState actualizado");

    // Actualiza WsContext y guarda metadatos
    {
        let ctx = state.ws_ctx.lock().await;
        *ctx.last_config.write().await = Some(serde_json::to_value(&cfg).unwrap_or(json!({})));
        *ctx.flight_id.write().await = Some(flight_id.clone());
        
        match ctx.questdb.save_experiment_metadata(&metadata).await {
            Ok(_) => info!("✅ Metadatos guardados exitosamente"),
            Err(e) => {
                error!("❌ Error guardando metadatos: {}", e);
            }
        }
        
        let ws_tx = &ctx.tx;
        let _ = ws_tx.send(json!({
            "type": "recording_started",
            "flight_id": &flight_id
        }).to_string());
    }

    Ok(Json(json!({
        "status": "ok",
        "flightId": flight_id
    })))
}

#[axum::debug_handler]
pub async fn stop_recording(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {

    let flight_id = {
        let ctx = state.ws_ctx.lock().await;
        let mut guard = ctx.flight_id.write().await;
        guard.take()
    };

    if let Some(fid) = flight_id {
        // Limpia AppState
        {
            let mut cur = state.current_config.write().await;
            *cur = None;
            let mut cfid = state.current_flight_id.write().await;
            *cfid = None;
        }

        {
            let ctx = state.ws_ctx.lock().await;
            let end_time = chrono::Utc::now();
            if let Err(e) = ctx.questdb.end_experiment(&fid, end_time).await {
                eprintln!("⚠️ Failed to update experiment end time: {}", e);
            }
            
            let ws_tx = &ctx.tx;
            let _ = ws_tx.send(json!({
                "type": "recording_stopped",
                "flight_id": &fid
            }).to_string());

            // Guarda evento (opcional) en DB
            let event = json!({ "event": "stop", "flightId": &fid }).to_string();
            if let Err(e) = ctx.questdb.insert_logger_config(&event).await {
                eprintln!("⚠️  {e}");
            }
        }

        Ok(Json(json!({
            "status": "stopped",
            "flightId": fid
        })))
    } else {
        Err((StatusCode::BAD_REQUEST, "No active recording to stop".into()))
    }
}

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("Internal server error: {0}")]
    Internal(String),
    #[error("Not found: {0}")]
    NotFound(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        };
        (status, message).into_response()
    }
}

#[derive(Deserialize)]
pub struct ListFlightsQuery { limit: Option<i64> }

#[derive(Serialize)]
pub struct FlightItem { flight_id: String, last_ts: String }

#[axum::debug_handler]
pub async fn list_flights(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListFlightsQuery>,
) -> Result<Json<Vec<FlightItem>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 1000);

    // 🔹 Snapshot rápido y soltar el lock
    let questdb = {
        let ctx = state.ws_ctx.lock().await;
        ctx.questdb.clone()
    };

    // Add a small delay to prevent overwhelming the database
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    match questdb.list_flights(limit).await {
        Ok(rows) => {
            let items: Vec<FlightItem> = rows.into_iter()
                .map(|(fid, ts)| FlightItem {
                    flight_id: fid,
                    last_ts: ts.to_rfc3339(),
                })
                .collect();
            Ok(Json(items))
        },
        Err(e) => {
            let error_msg = format!("Failed to retrieve flight list: {}", e);
            error!("{}", error_msg);
            
            // Return an empty list with a warning instead of an error if the table doesn't exist yet
            if e.to_string().contains("not exist") || e.to_string().contains("not found") {
                warn!("Database table or column not found, returning empty flight list");
                Ok(Json(Vec::new()))
            } else {
                Err(ApiError::Internal(error_msg))
            }
        }
    }
}

#[derive(Deserialize)]
pub struct SeriesQuery {
    fields: Option<String>,
    from: Option<String>,
    to: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize)]
pub struct SeriesPoint {
    ts: String,
    values: HashMap<String, f64>,
}

#[axum::debug_handler]
pub async fn get_flight_series(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
    Query(q): Query<SeriesQuery>,
) -> Result<Json<Vec<SeriesPoint>>, ApiError> {
    let ctx = state.ws_ctx.lock().await;

    let parse_dt = |s: &str| chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| {
            eprintln!("❌ Invalid date format: {e}");
            ApiError::Internal("Invalid date format".to_string())
        });

    let from = if let Some(from_str) = &q.from { Some(parse_dt(from_str)?) } else { None };
    let to   = if let Some(to_str)   = &q.to   { Some(parse_dt(to_str)?) }   else { None };

    let limit = q.limit.unwrap_or(50_000);
    let fields: Vec<String> = q.fields
        .as_ref()
        .map(|csv| csv.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_else(|| vec![
            "AngleRoll".into(), 
            "AnglePitch".into(), 
            "InputThrottle".into(),
            "tau_x".into(),
            "tau_y".into(),
            "tau_z".into()
        ]);

    if fields.is_empty() {
        return Err(ApiError::Internal("No fields specified".to_string()));
    }

    let points = ctx.questdb.fetch_flight_points(&fid, from, to, limit).await
        .map_err(|e| {
            eprintln!("❌ get_flight_series: {e}");
            ApiError::Internal("Failed to fetch flight data".to_string())
        })?;

    if points.is_empty() {
        return Err(ApiError::NotFound(format!("No data found for flight {}", fid)));
    }

    let mut out: Vec<SeriesPoint> = Vec::new();

    // Debug: Log the structure of the first point
    if let Some(first_point) = points.first() {
        //println!("First point structure: {:?}", first_point);
        if let Some(payload) = first_point.payload.get("payload").and_then(|v| v.as_object()) {
            println!("Available fields in first point: {:?}", payload.keys().collect::<Vec<_>>());
        }
    }

    for p in points {
        let mut map: HashMap<String, f64> = HashMap::new();
        
        // Try to get fields from the payload object if it exists
        if let Some(payload_obj) = p.payload.get("payload").and_then(|v| v.as_object()) {
            for f in &fields {
                if let Some(val) = payload_obj.get(f) {
                    if let Some(x) = val.as_f64() {
                        map.insert(f.clone(), x);
                    } else if let Some(xi) = val.as_i64() {
                        map.insert(f.clone(), xi as f64);
                    } else if let Some(xu) = val.as_u64() {
                        map.insert(f.clone(), xu as f64);
                    }
                }
            }
        } 
        // Also try to get fields directly from the root object
        else {
            for f in &fields {
                if let Some(val) = p.payload.get(f) {
                    if let Some(x) = val.as_f64() {
                        map.insert(f.clone(), x);
                    } else if let Some(xi) = val.as_i64() {
                        map.insert(f.clone(), xi as f64);
                    } else if let Some(xu) = val.as_u64() {
                        map.insert(f.clone(), xu as f64);
                    }
                }
            }
        }
        
        out.push(SeriesPoint { ts: p.ts.to_rfc3339(), values: map });
    }

    Ok(Json(out))
}

#[derive(Serialize)]
pub struct FlightSummary {
    flight_id: String,
    start_ts: String,
    end_ts: String,
    duration_sec: f64,
    max_roll: Option<f64>,
    max_pitch: Option<f64>,
    throttle_time_in_range_sec: f64,
    throttle_time_out_range_sec: f64,
}

#[derive(Deserialize)]
pub struct SummaryQuery {
    throttle_min: Option<f64>,
    throttle_max: Option<f64>,
}

#[axum::debug_handler]
pub async fn get_flight_summary(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
    Query(q): Query<SummaryQuery>,
) -> Result<Json<FlightSummary>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    let points = ctx.questdb.fetch_flight_points(&fid, None, None, 1_000_000).await
        .map_err(|e| {
            eprintln!("❌ get_flight_summary: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;

    if points.is_empty() {
        return Err(ApiError::NotFound(format!("Flight {} not found", fid)));
    }

    let start_ts = points.first().unwrap().ts;
    let end_ts = points.last().unwrap().ts;
    let duration = (end_ts - start_ts).num_milliseconds() as f64 / 1000.0;

    let thr_min = q.throttle_min.unwrap_or(1200.0);
    let thr_max = q.throttle_max.unwrap_or(2000.0);

    let mut max_roll: Option<f64> = None;
    let mut max_pitch: Option<f64> = None;
    let mut in_range = 0.0f64;
    let mut out_range = 0.0f64;

    for w in points.windows(2) {
        let a = &w[0];
        let b = &w[1];
        let dt = (b.ts - a.ts).num_milliseconds() as f64 / 1000.0;

        if let Some(obj) = a.payload.get("payload").and_then(|v| v.as_object()) {
            if let Some(v) = obj.get("AngleRoll").and_then(|x| x.as_f64()) {
                max_roll = Some(max_roll.map(|m| m.max(v.abs())).unwrap_or(v.abs()));
            }
            if let Some(v) = obj.get("AnglePitch").and_then(|x| x.as_f64()) {
                max_pitch = Some(max_pitch.map(|m| m.max(v.abs())).unwrap_or(v.abs()));
            }
            if let Some(th) = obj.get("InputThrottle").and_then(|x| x.as_f64()) {
                if th >= thr_min && th <= thr_max { in_range += dt; } else { out_range += dt; }
            }
        }
    }

    Ok(Json(FlightSummary {
        flight_id: fid,
        start_ts: start_ts.to_rfc3339(),
        end_ts: end_ts.to_rfc3339(),
        duration_sec: duration,
        max_roll,
        max_pitch,
        throttle_time_in_range_sec: in_range,
        throttle_time_out_range_sec: out_range,
    }))
}

#[derive(serde::Deserialize)]
pub struct IngestReq {
    records: Vec<serde_json::Value>,
    mode: Option<String>,
    ts_field: Option<String>,
    schema_version: Option<String>,
}

#[derive(serde::Serialize)]
pub struct IngestResp {
    status: String,
    inserted: usize,
    #[serde(rename = "flightId")]
    flight_id: String,
}

#[axum::debug_handler]
pub async fn ingest_points(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IngestReq>,
) -> Result<Json<IngestResp>, ApiError> {

    // 🟢 MÉTRICA 1: datos recibidos
    state.stats.ilp_enqueued.fetch_add(req.records.len() as u64, Ordering::Relaxed);

    info!(
        "/api/ingest: records={}, mode={:?}, ts_field={:?}, schema={:?}",
        req.records.len(), req.mode, req.ts_field, req.schema_version
    );

    if let Some(_first) = req.records.get(0) {
        //tracing::debug!("/api/ingest first record: {}", first);
    }

    let ctx = state.ws_ctx.lock().await;

    let fid = ctx.flight_id.read().await.clone()
        .ok_or_else(|| ApiError::NotFound("No active flight. Call /api/start first".into()))?;

    let inserted = ctx.questdb
        .ingest_telemetry_batch(
            &fid,
            req.schema_version.as_deref().unwrap_or("1"),
            req.mode.as_deref(),
            &req.records,
            req.ts_field.as_deref(),
        )
        .await
        .map_err(|e| {
            // 🔴 MÉTRICA 2: error
            state.stats.ilp_failed.fetch_add(1, Ordering::Relaxed);

            tracing::error!("ingest_points: {}", e);
            ApiError::Internal(format!("Failed to ingest telemetry: {}", e))
        })?;

    // 🟢 MÉTRICA 3: datos insertados correctamente
    state.stats.ilp_flushed.fetch_add(inserted as u64, Ordering::Relaxed);
    state.stats.mark_flush_now();
    
    info!("/api/ingest: inserted={} flight_id={}", inserted, fid);
    Ok(Json(IngestResp { 
        status: "ok".into(), 
        inserted, 
        flight_id: fid 
    }))
}

pub async fn get_stats(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let stats = &state.stats;

    Json(json!({
        "udp_received": stats.udp_received.load(Ordering::Relaxed),
        "selected_kept": stats.selected_kept.load(Ordering::Relaxed),
        "ilp_enqueued": stats.ilp_enqueued.load(Ordering::Relaxed),
        "ilp_flushed": stats.ilp_flushed.load(Ordering::Relaxed),
        "ilp_failed": stats.ilp_failed.load(Ordering::Relaxed),
        "channel_depth": stats.channel_depth.load(Ordering::Relaxed),
        "last_flush_ns": stats.last_flush_instant_ns.load(Ordering::Relaxed),
    }))
}