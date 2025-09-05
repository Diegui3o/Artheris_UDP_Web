// src/config/handlers.rs
use crate::config::metrics as met;
use crate::ws_server::http_server::{AppState, ApiError};
use axum::{extract::{Path, State}, Json};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize)]
pub struct FlightMetricsResponse {
    pub flight_id: String,
    pub start_ts: String,
    pub end_ts: String,
    pub duration_sec: f64,
    pub metrics: met::AngleMetrics,
    pub plot_fields: Vec<String>,
}

#[axum::debug_handler]
pub async fn get_flight_metrics(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<FlightMetricsResponse>, ApiError> {
    let ctx = state.ws_ctx.lock().await;

    // Trae puntos (columnas en el root del JSON)
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("❌ get_flight_metrics: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;

    // Si no hay puntos, devuelve 200 con métricas vacías (el front no queda en “Cargando…”)
    if points.len() < 2 {
        let now = chrono::Utc::now();
        let empty = met::AngleMetrics {
            rmse_roll: None, rmse_pitch: None,
            itae_roll: None, itae_pitch: None,
            mae_roll: None, mae_pitch: None,
            n_segments_used: 0,
            duration_sec: 0.0,
        };
        return Ok(Json(FlightMetricsResponse {
            flight_id: fid,
            start_ts: now.to_rfc3339(),
            end_ts: now.to_rfc3339(),
            duration_sec: 0.0,
            metrics: empty,
            plot_fields: met::EXTRA_PLOT_FIELDS.iter().map(|s| s.to_string()).collect(),
        }));
    }

    let start_ts = points.first().unwrap().ts;
    let end_ts   = points.last().unwrap().ts;
    let t0 = start_ts;

    // Prepara muestras leyendo del ROOT (no de payload interno)
    let mut samples = Vec::with_capacity(points.len());
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;

        let roll      = met::get_any(obj, met::FIELD_ROLL, met::ALT_ROLL);
        let des_roll  = met::get_any(obj, met::FIELD_DES_ROLL, met::ALT_DES_ROLL);
        let pitch     = met::get_any(obj, met::FIELD_PITCH, met::ALT_PITCH);
        let des_pitch = met::get_any(obj, met::FIELD_DES_PITCH, met::ALT_DES_PITCH);

        samples.push(met::AngleSample { t_rel, roll, des_roll, pitch, des_pitch });
    }

    let metrics = met::compute_angle_metrics(&samples);
    let duration_sec = (end_ts - start_ts).num_milliseconds() as f64 / 1000.0;

    Ok(Json(FlightMetricsResponse {
        flight_id: fid,
        start_ts: start_ts.to_rfc3339(),
        end_ts: end_ts.to_rfc3339(),
        duration_sec,
        metrics,
        plot_fields: met::EXTRA_PLOT_FIELDS.iter().map(|s| s.to_string()).collect(),
    }))
}
