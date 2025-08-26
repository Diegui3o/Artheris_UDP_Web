pub mod http_server;
pub mod questdb;
pub mod ilp;  
pub mod server;

pub use server::{start_ws_server, WsContext};
pub use questdb::OptionalDb;

use axum::{routing::{get, post}, extract::{State, Path, Query}, Json, Router};
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

async fn apply_config(
    State(ctx): State<WsContext>,
    Json(cfg): Json<LoggerConfig>,
) -> Json<ApiOk> {
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

async fn start_recording(
    State(ctx): State<WsContext>,
    Json(cfg): Json<serde_json::Value>,
) -> Json<StartResp> {
    let flight_id = format!("flt_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S"));
    {
        let mut guard = ctx.flight_id.write().await;
        *guard = Some(flight_id.clone());
    }

    // Intenta guardar el evento de inicio (opcional)
    let event = serde_json::json!({
        "event": "start",
        "flightId": &flight_id,
        "config": cfg
    }).to_string();
    
    if let Err(e) = ctx.questdb.insert_logger_config(&event).await {
        eprintln!("⚠️  {e}");
    }
    
    Json(StartResp { status: "ok".into(), flightId: flight_id })
}

async fn stop_recording(
    State(ctx): State<WsContext>,
) -> Json<ApiOk> {
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
        // existentes:
        .route("/api/logger/config", post(apply_config))
        .route("/api/recordings/start", post(start_recording))
        .route("/api/recordings/stop", post(stop_recording))
        // NUEVOS análisis:
        .route("/api/flights", get(list_flights))
        .route("/api/flights/:id/series", get(get_flight_series))
        .route("/api/flights/:id/summary", get(get_flight_summary))
        .with_state(ctx)
        .layer(cors);

    let addr = std::net::SocketAddr::from(([0,0,0,0], 3000));
    println!("🌐 HTTP listening on http://{addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}

#[derive(Deserialize)]
struct ListFlightsQuery { limit: Option<i64> }

#[derive(Serialize)]
struct FlightItem { flight_id: String, last_ts: String }

async fn list_flights(State(ctx): State<WsContext>, Query(q): Query<ListFlightsQuery>) -> Json<Vec<FlightItem>> {
    let limit = q.limit.unwrap_or(50);
    let mut items = Vec::new();
    match ctx.questdb.list_flights(limit).await {
        Ok(rows) => {
            for (fid, ts) in rows {
                items.push(FlightItem { flight_id: fid, last_ts: ts.to_rfc3339() });
            }
        }
        Err(e) => eprintln!("❌ list_flights: {e}"),
    }
    Json(items)
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

async fn get_flight_series(
    State(ctx): State<WsContext>,
    Path(fid): Path<String>,
    Query(q): Query<SeriesQuery>,
) -> Json<Vec<SeriesPoint>> {
    // parse fechas
    let parse_dt = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| dt.with_timezone(&chrono::Utc));
    let from = q.from.as_deref().and_then(parse_dt);
    let to = q.to.as_deref().and_then(parse_dt);

    let limit = q.limit.unwrap_or(50_000);
    let fields: Vec<String> = q.fields
        .as_ref()
        .map(|csv| csv.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_else(|| vec!["AngleRoll".into(),"AnglePitch".into(),"InputThrottle".into()]);

    let mut out = Vec::new();

    match ctx.questdb.fetch_flight_points(&fid, from, to, limit).await {
        Ok(points) => {
            for p in points {
                // payload → {"type":"telemetry","payload":{ ...pares clave:valor... }}
                let mut map = HashMap::new();
                let inner = p.payload.get("payload").and_then(|v| v.as_object());
                if let Some(obj) = inner {
                    for f in &fields {
                        if let Some(val) = obj.get(f) {
                            if let Some(x) = val.as_f64() {
                                map.insert(f.clone(), x);
                            } else if let Some(xi) = val.as_i64() { map.insert(f.clone(), xi as f64); }
                            else if let Some(xu) = val.as_u64() { map.insert(f.clone(), xu as f64); }
                        }
                    }
                }
                out.push(SeriesPoint { ts: p.ts.to_rfc3339(), values: map });
            }
        }
        Err(e) => eprintln!("❌ get_flight_series: {e}"),
    }
    Json(out)
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

async fn get_flight_summary(
    State(ctx): State<WsContext>,
    Path(fid): Path<String>,
    Query(q): Query<SummaryQuery>,
) -> Json<Option<FlightSummary>> {
    let points = match ctx.questdb.fetch_flight_points(&fid, None, None, 1_000_000).await {
        Ok(v) => v,
        Err(e) => { eprintln!("❌ get_flight_summary: {e}"); return Json(None); }
    };
    if points.is_empty() { return Json(None); }

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

        let inner = a.payload.get("payload").and_then(|v| v.as_object());
        if let Some(obj) = inner {
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

    Json(Some(FlightSummary {
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