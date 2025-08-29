pub mod http_server;
pub mod questdb;
pub mod ilp;  
pub mod server;

use crate::ws_server::questdb::probe_sql_insert;

pub use server::{start_ws_server, WsContext};
pub use questdb::OptionalDb;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use tower_http::trace::TraceLayer;
use std::sync::Arc;
use tokio::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Internal server error: {0}")]
    Internal(String),
    #[error("Not found: {0}")]
    NotFound(String),
}

#[axum::debug_handler]
pub async fn ingest_sample(
    State(ctx): State<Arc<Mutex<WsContext>>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ctx = ctx.lock().await;
    let fid = ctx
        .flight_id
        .read()
        .await
        .clone()
        .ok_or_else(|| ApiError::NotFound("No active flight. Start one first.".into()))?;

    // Muestra de record
    let sample = serde_json::json!({
        "time": chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0), // ns
        "InputThrottle": 1500,
        "AngleRoll": 1.23,
        "AnglePitch": -0.5,
        "RateYaw": 0.02
    });

    let inserted = ctx
        .questdb
        .ingest_telemetry_batch(&fid, "1", None, std::slice::from_ref(&sample), Some("time"))
        .await
        .map_err(|e| ApiError::Internal(format!("ingest_sample failed: {e}")))?;

    Ok(Json(serde_json::json!({ "status":"ok", "inserted": inserted, "flightId": fid })))
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, error_message) = match self {
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
        };

        let body = Json(serde_json::json!({ "error": error_message }));
        (status, body).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;
use std::time::Duration;
use tower_http::cors::{CorsLayer, Any};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ====== HTTP payloads ======
#[derive(Debug, Deserialize)]
struct LoggerConfig {
    #[serde(flatten)]
    rest: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct ApiOk { status: String }
#[derive(Debug, Serialize)]
struct StartResp { status: String, flightId: String }

pub async fn apply_config(
    State(ctx): State<Arc<Mutex<WsContext>>>,
    Json(cfg): Json<LoggerConfig>,
) -> impl IntoResponse {
    let ctx = ctx.lock().await;
    // Guarda para referencia
    {
        let mut last = ctx.last_config.write().await;
        *last = Some(cfg.rest.clone());
    }

    // Intenta guardar en QuestDB (opcional)
    match ctx.questdb.insert_logger_config(&cfg.rest.to_string()).await {
        Ok(_) => {},
        Err(e) => eprintln!("⚠️  {e}"),
    }

    Json(ApiOk { status: "ok".into() })
}

pub async fn start_recording(
    State(ctx): State<Arc<Mutex<WsContext>>>,
    Json(cfg): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ctx = ctx.lock().await;
    let flight_id = format!("flt_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S"));
    tracing::info!("start_recording: new flight_id={}", flight_id);
    if let Some(obj) = cfg.as_object() {
        tracing::debug!("start_recording cfg keys={:?}", obj.keys().collect::<Vec<_>>());
    }
    {
        let mut guard = ctx.flight_id.write().await;
        *guard = Some(flight_id.clone());
    }
    // guarda evento (como ya tenías)
    Json(StartResp { status: "ok".into(), flightId: flight_id })
}


pub async fn stop_recording(
    State(ctx): State<Arc<Mutex<WsContext>>>,
) -> impl IntoResponse {
    let ctx = ctx.lock().await;
    let fid = {
        let mut guard = ctx.flight_id.write().await;
        guard.take().unwrap_or_else(|| "none".into())
    };
    
    // Intenta guardar el evento de parada (opcional)
    let event = serde_json::json!({
        "event": "stop",
        "flightId": fid
    }).to_string();
    
    if let Err(e) = ctx.questdb.insert_logger_config(&event).await {
        eprintln!("⚠️  {e}");
    }
    
    Json(ApiOk { status: "ok".into() })
}

// Lanza el servidor HTTP en :3000
pub async fn start_http_server(ctx: WsContext) -> anyhow::Result<()> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
        .max_age(Duration::from_secs(3600));

        let app = Router::new()
        .route("/api/config", post(apply_config))
        .route("/api/start", post(start_recording))
        .route("/api/stop", post(stop_recording))
        .route("/api/flights", get(list_flights))
        .route("/api/flights/:id/series", get(get_flight_series))
        .route("/api/flights/:id/summary", get(get_flight_summary))
        .route("/api/ingest", post(ingest_points))
        .route("/api/probe-sql", post(probe_sql_insert))
        .with_state(Arc::new(Mutex::new(ctx)))
        .layer(cors)
        .layer(TraceLayer::new_for_http()); // 👈 último

    let addr = std::net::SocketAddr::from(([0,0,0,0], 3000));
    tracing::info!("🌐 HTTP listening on http://{addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}

#[derive(Deserialize)]
struct ListFlightsQuery { limit: Option<i64> }

#[derive(Serialize)]
struct FlightItem { flight_id: String, last_ts: String }

#[axum::debug_handler]
pub async fn list_flights(
    State(ctx): State<Arc<Mutex<WsContext>>>,
    Query(q): Query<ListFlightsQuery>,
) -> Result<Json<Vec<FlightItem>>, ApiError> {
    let ctx = ctx.lock().await;
    let limit = q.limit.unwrap_or(50);
    let rows = ctx.questdb.list_flights(limit).await
        .map_err(|e| {
            eprintln!("❌ list_flights: {e}");
            ApiError::Internal("Failed to fetch flights".to_string())
        })?;

    let items: Vec<FlightItem> = rows.into_iter().map(|(fid, ts)| {
        FlightItem {
            flight_id: fid,
            last_ts: ts.to_rfc3339(),
        }
    }).collect();
    
    Ok(Json(items))
}

#[derive(Deserialize)]
struct SeriesQuery {
    // campos de interés ej: AngleRoll,AnglePitch,InputThrottle
    fields: Option<String>,
    from: Option<String>,
    to: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize)]
struct SeriesPoint {
    ts: String,
    values: HashMap<String, f64>,
}

#[axum::debug_handler]
pub async fn get_flight_series(
    State(ctx): State<Arc<Mutex<WsContext>>>,
    Path(fid): Path<String>,
    Query(q): Query<SeriesQuery>,
) -> Result<Json<Vec<SeriesPoint>>, ApiError> {
    let ctx = ctx.lock().await;
    // Parse dates
    let parse_dt = |s: &str| chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| {
            eprintln!("❌ Invalid date format: {e}");
            ApiError::Internal("Invalid date format".to_string())
        });

    let from = if let Some(from_str) = &q.from {
        Some(parse_dt(from_str)?)
    } else {
        None
    };

    let to = if let Some(to_str) = &q.to {
        Some(parse_dt(to_str)?)
    } else {
        None
    };

    let limit = q.limit.unwrap_or(50_000);
    let fields: Vec<String> = q.fields
        .as_ref()
        .map(|csv| csv.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_else(|| vec!["AngleRoll".into(), "AnglePitch".into(), "InputThrottle".into()]);
        
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
    
    for p in points {
        // payload → {"type":"telemetry","payload":{ ...pares clave:valor... }}
        let mut map = HashMap::new();
        let inner = p.payload.get("payload").and_then(|v| v.as_object());
        if let Some(obj) = inner {
            for f in &fields {
                if let Some(val) = obj.get(f) {
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
        out.push(SeriesPoint { 
            ts: p.ts.to_rfc3339(), 
            values: map 
        });
    }
    
    Ok(Json(out))
}

#[derive(Serialize)]
struct FlightSummary {
    flight_id: String,
    start_ts: String,
    end_ts: String,
    duration_sec: f64,
    // ejemplo de métricas
    max_roll: Option<f64>,
    max_pitch: Option<f64>,
    throttle_time_in_range_sec: f64,
    throttle_time_out_range_sec: f64,
}

#[derive(Deserialize)]
struct SummaryQuery {
    throttle_min: Option<f64>,
    throttle_max: Option<f64>,
}

#[derive(serde::Deserialize)]
struct IngestReq {
    // Lote de samples. Cada objeto es tu payload con campos: AngleRoll, InputThrottle, etc.
    records: Vec<serde_json::Value>,
    // Opcional: si quieres anotar el “modo” como tag
    mode: Option<String>,
    // Nombre del campo de tiempo en cada record (ej. "time"); si se omite, se usa “now”
    ts_field: Option<String>,
    // Versión de esquema (string porque la columna es SYMBOL)
    schema_version: Option<String>,
}

#[derive(serde::Serialize)]
struct IngestResp {
    status: String,
    inserted: usize,
    flightId: String,
}

pub async fn ingest_points(
    State(ctx): State<Arc<Mutex<WsContext>>>,
    Json(req): Json<IngestReq>,
) -> Result<Json<IngestResp>, ApiError> {
    tracing::info!(
        "/api/ingest: records={}, mode={:?}, ts_field={:?}, schema={:?}",
        req.records.len(),
        req.mode,
        req.ts_field,
        req.schema_version
    );
    if let Some(first) = req.records.get(0) {
        tracing::debug!("/api/ingest first record: {}", first);
    }

    let ctx = ctx.lock().await;
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
            tracing::error!("ingest_points: {}", e);
            ApiError::Internal(format!("Failed to ingest telemetry: {}", e))
        })?;

    tracing::info!("/api/ingest: inserted={} flight_id={}", inserted, fid);

    Ok(Json(IngestResp { status: "ok".into(), inserted, flightId: fid }))
}

#[axum::debug_handler]
pub async fn get_flight_summary(
    State(ctx): State<Arc<Mutex<WsContext>>>,
    Path(fid): Path<String>,
    Query(q): Query<SummaryQuery>,
) -> Result<Json<FlightSummary>, ApiError> {
    let ctx = ctx.lock().await;
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

    let mut max_roll = None::<f64>;
    let mut max_pitch = None::<f64>;
    let mut in_range = 0.0f64;
    let mut out_range = 0.0f64;

    // integramos por “tramos” (asumiendo frecuencia relativamente uniforme)
    for w in points.windows(2) {
        let a = &w[0];
        let b = &w[1];
        let dt = (b.ts - a.ts).num_milliseconds() as f64 / 1000.0;
    
        // ✅ Aquí debes usar `a.payload`, no `p.payload`
        if let Some(obj) = a.payload.as_object() {
            if let Some(v) = obj.get("AngleRoll").and_then(|x| x.as_f64()) {
                max_roll = Some(max_roll.map(|m| m.max(v.abs())).unwrap_or(v.abs()));
            }
            if let Some(v) = obj.get("AnglePitch").and_then(|x| x.as_f64()) {
                max_pitch = Some(max_pitch.map(|m| m.max(v.abs())).unwrap_or(v.abs()));
            }
            if let Some(th) = obj.get("InputThrottle").and_then(|x| x.as_f64()) {
                if th >= thr_min && th <= thr_max {
                    in_range += dt;
                } else {
                    out_range += dt;
                }
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