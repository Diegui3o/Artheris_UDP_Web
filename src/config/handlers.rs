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
            variance_roll: None, variance_pitch: None,
            std_dev_roll: None, std_dev_pitch: None,
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
        let kalman_roll = met::get_any(obj, "KalmanAngleRoll", &[]);
        let pitch     = met::get_any(obj, met::FIELD_PITCH, met::ALT_PITCH);
        let des_pitch = met::get_any(obj, met::FIELD_DES_PITCH, met::ALT_DES_PITCH);
        let kalman_pitch = met::get_any(obj, "KalmanAnglePitch", &[]);

        samples.push(met::AngleSample { t_rel, roll, des_roll, kalman_roll, pitch, des_pitch, kalman_pitch });
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

#[axum::debug_handler]
pub async fn get_error_comparison(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<met::ErrorComparisonMetrics>, ApiError> {
    let ctx = state.ws_ctx.lock().await;

    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("❌ get_error_comparison: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;

    if points.len() < 2 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data", fid)));
    }

    let t0 = points[0].ts;
    let mut samples = Vec::with_capacity(points.len());

    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;

        let roll = met::get_any(obj, met::FIELD_ROLL, met::ALT_ROLL);
        let des_roll = met::get_any(obj, met::FIELD_DES_ROLL, met::ALT_DES_ROLL);
        let kalman_roll = met::get_any(obj, "KalmanAngleRoll", &[]);

        samples.push(met::AngleSample {
            t_rel,
            roll,
            des_roll,
            kalman_roll,
            pitch: None,
            des_pitch: None,
            kalman_pitch: None,
        });
    }

    let comparison = met::compute_error_comparison(&samples);
    Ok(Json(comparison))
}

#[axum::debug_handler]
pub async fn get_flight_metrics_full(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<met::FullFlightMetrics>, ApiError> {
    let ctx = state.ws_ctx.lock().await;

    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("❌ get_flight_metrics_full: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;

    if points.len() < 2 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data", fid)));
    }

    let start_ts = points.first().unwrap().ts;
    let end_ts = points.last().unwrap().ts;
    let t0 = start_ts;

    // Preparar muestras
    let mut samples = Vec::with_capacity(points.len());
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;

        let roll = met::get_any(obj, met::FIELD_ROLL, met::ALT_ROLL);
        let des_roll = met::get_any(obj, met::FIELD_DES_ROLL, met::ALT_DES_ROLL);
        let kalman_roll = met::get_any(obj, "KalmanAngleRoll", &[]);
        let pitch = met::get_any(obj, met::FIELD_PITCH, met::ALT_PITCH);
        let des_pitch = met::get_any(obj, met::FIELD_DES_PITCH, met::ALT_DES_PITCH);
        let kalman_pitch = met::get_any(obj, "KalmanAnglePitch", &[]);

        samples.push(met::AngleSample { 
            t_rel, 
            roll, des_roll, kalman_roll, 
            pitch, des_pitch, kalman_pitch 
        });
    }

    let metrics = met::compute_full_flight_metrics(&fid, &samples, start_ts, end_ts);
    
    Ok(Json(metrics))
}