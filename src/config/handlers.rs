use crate::config::metrics as met;
use crate::ws_server::http_server::{AppState, ApiError};
use axum::{extract::{Path, State}, Json};
use serde::Serialize;
use std::sync::Arc;
use crate::config::metrics::{self, get_any};

#[derive(Debug, Serialize)]
pub struct FlightMetricsResponse {
    pub flight_id: String,
    pub start_ts: String,
    pub end_ts: String,
    pub duration_sec: f64,
    pub metrics: met::AngleMetrics,
    pub plot_fields: Vec<String>,
}
use crate::analysis::fft::{compute_spectrum, Spectrum as FftSpectrum};
use crate::config::spectrum_types::{FlightSpectrum, Spectrum, Peak, Correlation};

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

#[axum::debug_handler]
pub async fn get_flight_spectrum(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<FlightSpectrum>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    
    // Obtener todos los puntos del vuelo
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("❌ get_flight_spectrum: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;
    
    if points.len() < 10 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data ({} points)", fid, points.len())));
    }
    
    // Calcular frecuencia de muestreo automáticamente a partir de los timestamps
    let sample_rate_hz = if points.len() > 1 {
        let mut intervals = Vec::new();
        for i in 1..points.len() {
            let dt = (points[i].ts - points[i-1].ts).num_milliseconds() as f64 / 1000.0;
            if dt > 0.0 && dt < 1.0 {  // ignorar intervalos irrazonables
                intervals.push(dt);
            }
        }
        
        if intervals.is_empty() {
            25.0  // fallback a 25Hz
        } else {
            let avg_interval = intervals.iter().sum::<f64>() / intervals.len() as f64;
            1.0 / avg_interval
        }
    } else {
        25.0  // fallback
    };

    println!("📊 Frecuencia de muestreo detectada: {:.1} Hz", sample_rate_hz);
    
    // Extraer señales
    let mut error_signal = Vec::new();      // phi_ref - KalmanAngleRoll
    let mut motor_signal = Vec::new();       // promedio de motores
    let mut acc_x_signal = Vec::new();
    let mut acc_y_signal = Vec::new();
    let mut acc_z_signal = Vec::new();
    
    for p in &points {
        let obj = &p.payload;
        
        // Error: phi_ref - KalmanAngleRoll
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let kalman_roll = met::get_any(obj, "KalmanAngleRoll", &[]);
        
        if let (Some(phi), Some(kalman)) = (phi_ref, kalman_roll) {
            error_signal.push(phi - kalman);
        }
        
        // Motores: promedio de MotorInput1-4
        let m1 = met::get_any(obj, "MotorInput1", &[]);
        let m2 = met::get_any(obj, "MotorInput2", &[]);
        let m3 = met::get_any(obj, "MotorInput3", &[]);
        let m4 = met::get_any(obj, "MotorInput4", &[]);
        
        let motor_vals: Vec<f64> = [m1, m2, m3, m4].iter().filter_map(|&x| x).collect();
        if !motor_vals.is_empty() {
            let avg = motor_vals.iter().sum::<f64>() / motor_vals.len() as f64;
            motor_signal.push(avg);
        }
        
        // Acelerómetros
        if let Some(acc_x) = met::get_any(obj, "AccX", &[]) {
            acc_x_signal.push(acc_x);
        }
        if let Some(acc_y) = met::get_any(obj, "AccY", &[]) {
            acc_y_signal.push(acc_y);
        }
        if let Some(acc_z) = met::get_any(obj, "AccZ", &[]) {
            acc_z_signal.push(acc_z);
        }
    }
    
    // Función helper para crear Spectrum
    let create_spectrum = |signal: &[f64], name: &str| -> Spectrum {
        if signal.len() < 4 {
            return Spectrum {
                frequencies_hz: Vec::new(),
                magnitudes: Vec::new(),
                dominant_peaks: Vec::new(),
            };
        }
        
        let fft_result = compute_spectrum(signal, sample_rate_hz, 5);
        
        let peaks: Vec<Peak> = fft_result.dominant_peaks
            .iter()
            .map(|(freq, mag)| Peak {
                frequency_hz: *freq,
                magnitude: *mag,
            })
            .collect();
        
        Spectrum {
            frequencies_hz: fft_result.frequencies_hz,
            magnitudes: fft_result.magnitudes,
            dominant_peaks: peaks,
        }
    };
    
    // Calcular espectros
    let error_spectrum = create_spectrum(&error_signal, "error");
    let motors_spectrum = create_spectrum(&motor_signal, "motors");
    let acc_x_spectrum = create_spectrum(&acc_x_signal, "acc_x");
    let acc_y_spectrum = create_spectrum(&acc_y_signal, "acc_y");
    let acc_z_spectrum = create_spectrum(&acc_z_signal, "acc_z");
    
    // Correlacionar picos
    let mut correlations = Vec::new();
    
    // Buscar frecuencias comunes entre error y motores
    let error_peaks: Vec<f64> = error_spectrum.dominant_peaks.iter()
        .map(|p| p.frequency_hz)
        .collect();
    let motor_peaks: Vec<f64> = motors_spectrum.dominant_peaks.iter()
        .map(|p| p.frequency_hz)
        .collect();
    
    for freq in &error_peaks {
        if motor_peaks.iter().any(|mf| (mf - freq).abs() < 0.5) {
            correlations.push(Correlation {
                frequency_hz: *freq,
                sources: vec!["error".to_string(), "motors".to_string()],
                description: format!("Pico en {:.1} Hz aparece tanto en error como en motores", freq),
            });
        }
    }
    
    let flight_spectrum = FlightSpectrum {
        flight_id: fid,
        sample_rate_hz,
        sample_count: points.len(),
        error_spectrum,
        motors_spectrum,
        acc_x_spectrum,
        acc_y_spectrum,
        acc_z_spectrum,
        correlations,
    };
    
    Ok(Json(flight_spectrum))
}