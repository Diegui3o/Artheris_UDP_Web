use serde::Serialize;
use std::sync::Arc;
use axum::{extract::{Path, State}, Json};

use crate::ws_server::http_server::{AppState, ApiError};

/// Claves usadas para métricas de ángulos (ajústalas si tus nombres varían)
pub const FIELD_ROLL: &str = "AngleRoll";
pub const FIELD_PITCH: &str = "AnglePitch";
pub const FIELD_DES_ROLL: &str = "DesiredAngleRoll";
pub const FIELD_DES_PITCH: &str = "DesiredAnglePitch";

/// Campos que quieres graficar en la UI (preset para series)
pub const EXTRA_PLOT_FIELDS: &[&str] = &[
    "AccX", "AccY", "AccZ",
    "DesiredAnglePitch", "DesiredAngleRoll",
    "DesiredRateYaw",
    "g1", "g2",
    "k1", "k2",
    "m1", "m2",
    "tau_x", "tau_y", "tau_z",
];

/// Muestra “preprocesada” para el cálculo (ya en t_rel y con valores opcionales)
#[derive(Debug, Clone)]
pub struct AngleSample {
    /// tiempo relativo [s] desde el inicio del vuelo
    pub t_rel: f64,
    pub roll: Option<f64>,
    pub des_roll: Option<f64>,
    pub pitch: Option<f64>,
    pub des_pitch: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AngleMetrics {
    pub rmse_roll: Option<f64>,
    pub rmse_pitch: Option<f64>,
    pub itae_roll: Option<f64>,
    pub itae_pitch: Option<f64>,
    pub mae_roll: Option<f64>,
    pub mae_pitch: Option<f64>,
    pub n_segments_used: usize,
    pub duration_sec: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlightMetricsResponse {
    pub flight_id: String,
    pub start_ts: String,
    pub end_ts: String,
    pub duration_sec: f64,
    pub metrics: AngleMetrics,
    pub plot_fields: Vec<String>,
}

#[axum::debug_handler]
pub async fn get_flight_metrics(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<FlightMetricsResponse>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| ApiError::Internal(format!("DB error: {e}")))?;

    if points.len() < 2 {
        return Err(ApiError::NotFound(format!("No data for flight {fid}")));
    }

    let t0 = points.first().unwrap().ts;
    let mut samples = Vec::with_capacity(points.len());

    let get_f64 = |obj: &serde_json::Value, k: &str| -> Option<f64> {
        obj.get(k)
            .and_then(|v| v.as_f64()
                .or_else(|| v.as_i64().map(|x| x as f64))
                .or_else(|| v.as_u64().map(|x| x as f64)))
    };

    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload; // columnas al nivel raíz
        samples.push(AngleSample {
            t_rel,
            roll:      get_f64(obj, FIELD_ROLL),
            des_roll:  get_f64(obj, FIELD_DES_ROLL),
            pitch:     get_f64(obj, FIELD_PITCH),
            des_pitch: get_f64(obj, FIELD_DES_PITCH),
        });
    }

    let metrics = compute_angle_metrics(&samples);

    Ok(Json(FlightMetricsResponse {
        flight_id: fid,
        start_ts: points.first().unwrap().ts.to_rfc3339(),
        end_ts:   points.last().unwrap().ts.to_rfc3339(),
        duration_sec: (points.last().unwrap().ts - points.first().unwrap().ts)
            .num_milliseconds() as f64 / 1000.0,
        metrics,
        plot_fields: EXTRA_PLOT_FIELDS.iter().map(|s| s.to_string()).collect(),
    }))
}

pub fn compute_angle_metrics(samples: &[AngleSample]) -> AngleMetrics {
    if samples.len() < 2 {
        return AngleMetrics { rmse_roll: None, rmse_pitch: None, itae_roll: None, itae_pitch: None, mae_roll: None, mae_pitch: None, n_segments_used: 0, duration_sec: 0.0 };
    }

    let duration_sec = samples.last().unwrap().t_rel - samples.first().unwrap().t_rel;
    if duration_sec <= 0.0 {
        return AngleMetrics { rmse_roll: None, rmse_pitch: None, itae_roll: None, itae_pitch: None, mae_roll: None, mae_pitch: None, n_segments_used: 0, duration_sec: 0.0 };
    }

    let mut sum_abs_roll_dt = 0.0;
    let mut sum_abs_pitch_dt = 0.0;
    let mut sum_sq_roll_dt = 0.0;
    let mut sum_sq_pitch_dt = 0.0;
    let mut sum_itae_roll = 0.0;
    let mut sum_itae_pitch = 0.0;
    let mut used = 0usize;

    for w in samples.windows(2) {
        let a = &w[0];
        let b = &w[1];
        let dt = (b.t_rel - a.t_rel).max(0.0);
        if dt <= 0.0 { continue; }

        // Roll (solo punto izquierdo)
        if let (Some(r_a), Some(dr_a)) = (a.roll, a.des_roll) {
            let e = r_a - dr_a;
            sum_abs_roll_dt += e.abs() * dt;
            sum_sq_roll_dt  += e * e * dt;
            sum_itae_roll   += a.t_rel * e.abs() * dt;
            used += 1;
        }

        // Pitch (solo punto izquierdo)
        if let (Some(p_a), Some(dp_a)) = (a.pitch, a.des_pitch) {
            let e = p_a - dp_a;
            sum_abs_pitch_dt += e.abs() * dt;
            sum_sq_pitch_dt  += e * e * dt;
            sum_itae_pitch   += a.t_rel * e.abs() * dt;
            used += 1;
        }
    }

    let mae_roll  = (used > 0).then_some(sum_abs_roll_dt / duration_sec);
    let mae_pitch = (used > 0).then_some(sum_abs_pitch_dt / duration_sec);
    let rmse_roll = (used > 0).then_some((sum_sq_roll_dt / duration_sec).sqrt());
    let rmse_pitch= (used > 0).then_some((sum_sq_pitch_dt / duration_sec).sqrt());
    let itae_roll = (used > 0).then_some(sum_itae_roll);
    let itae_pitch= (used > 0).then_some(sum_itae_pitch);

    AngleMetrics { rmse_roll, rmse_pitch, itae_roll, itae_pitch, mae_roll, mae_pitch, n_segments_used: used, duration_sec }
}
