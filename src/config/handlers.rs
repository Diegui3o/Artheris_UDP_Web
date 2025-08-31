use crate::config::metrics as met;
use crate::ws_server::http_server::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use std::sync::Arc;
use crate::ws_server::http_server::ApiError;

use serde::Serialize;

/// Respuesta con métricas de vuelo calculadas
#[derive(Debug, Serialize)]
pub struct FlightMetricsResponse {
    /// ID del vuelo
    pub flight_id: String,
    /// Marca de tiempo de inicio del vuelo
    pub start_ts: String,
    /// Marca de tiempo de fin del vuelo
    pub end_ts: String,
    /// Duración del vuelo en segundos
    pub duration_sec: f64,
    /// Métricas de ángulos calculadas
    pub metrics: met::AngleMetrics,
    /// Campos sugeridos para graficar (útil para que el front llame a /series)
    pub plot_fields: Vec<String>,
}

#[axum::debug_handler]
pub async fn get_flight_metrics(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<FlightMetricsResponse>, ApiError> {
    let ctx = state.ws_ctx.lock().await;

    // Trae todos los puntos del vuelo
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("❌ get_flight_metrics: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;

    if points.is_empty() {
        return Err(ApiError::NotFound(format!("Flight {} not found", fid)));
    }

    let start_ts = points.first().unwrap().ts;
    let end_ts   = points.last().unwrap().ts;
    let t0 = start_ts;

    // Prepara muestras para el cómputo (t_rel en segundos + valores)
    let mut samples: Vec<met::AngleSample> = Vec::with_capacity(points.len());

    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;

        // payload → {"type":"telemetry","payload":{ ... pares clave:valor ... }}
        let obj = p.payload
            .get("payload")
            .and_then(|v| v.as_object());

        let mut roll: Option<f64> = None;
        let mut des_roll: Option<f64> = None;
        let mut pitch: Option<f64> = None;
        let mut des_pitch: Option<f64> = None;

        if let Some(map) = obj {
            // helper inline para extraer numéricos robustamente
            let mut get = |k: &str| -> Option<f64> {
                map.get(k)
                    .and_then(|v| v.as_f64()
                        .or_else(|| v.as_i64().map(|x| x as f64))
                        .or_else(|| v.as_u64().map(|x| x as f64)))
            };

            roll      = get(met::FIELD_ROLL);
            des_roll  = get(met::FIELD_DES_ROLL);
            pitch     = get(met::FIELD_PITCH);
            des_pitch = get(met::FIELD_DES_PITCH);
        }

        samples.push(met::AngleSample {
            t_rel,
            roll,
            des_roll,
            pitch,
            des_pitch,
        });
    }

    let metrics = met::compute_angle_metrics(&samples);

    Ok(Json(FlightMetricsResponse {
        flight_id: fid,
        start_ts: start_ts.to_rfc3339(),
        end_ts: end_ts.to_rfc3339(),
        duration_sec: metrics.duration_sec,
        metrics,
        plot_fields: met::EXTRA_PLOT_FIELDS.iter().map(|s| s.to_string()).collect(),
    }))
}
