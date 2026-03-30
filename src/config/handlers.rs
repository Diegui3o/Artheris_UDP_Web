use crate::config::metrics as met;
use crate::ws_server::http_server::{AppState, ApiError};
use axum::{extract::{Path, State}, Json};
use serde::Serialize;
use std::sync::Arc;
use crate::ws_server::questdb::FlightPoint;
use crate::config::metrics::AngleSample;
#[derive(Debug, Serialize)]
pub struct FlightMetricsResponse {
    pub flight_id: String,
    pub start_ts: String,
    pub end_ts: String,
    pub duration_sec: f64,
    pub metrics: met::AngleMetrics,
    pub plot_fields: Vec<String>,
}
use crate::analysis::fft::compute_spectrum;
use crate::config::spectrum_types::{FlightSpectrum, Spectrum, Peak, Correlation};

use crate::analysis::uncertainty::{
    UncertaintySource, ValidationResult, DistributionType,
    monte_carlo_simulation, create_uncertainty_budget,
};
use crate::config::uncertainty_types::UncertaintyResponse;
use crate::analysis::anomaly::analyze_flight_anomalies;
use crate::config::anomaly_types::AnomalyResponse;
use crate::analysis::correlation::analyze_correlations;
use crate::config::correlation_types::CorrelationResponse;
use crate::analysis::trend::analyze_trends;
use crate::config::trend_types::TrendResponse;
use crate::analysis::recommendations::generate_recommendations;
use crate::config::recommendation_types::RecommendationsResponse;
use crate::analysis::score::compute_quality_score;
use crate::config::score_types::ScoreResponse;
use crate::analysis::historical::{compare_flight_with_historical, get_historical_metrics, store_flight_metrics};
use crate::config::historical_types::HistoricalComparison;

// Función auxiliar para calcular frecuencia de muestreo
fn calculate_sample_rate(points: &[crate::ws_server::questdb::FlightPoint]) -> f64 {
    if points.len() < 2 {
        return 25.0;
    }
    let mut intervals = Vec::new();
    for i in 1..points.len() {
        let dt = (points[i].ts - points[i-1].ts).num_milliseconds() as f64 / 1000.0;
        if dt > 0.0 && dt < 1.0 {
            intervals.push(dt);
        }
    }
    if intervals.is_empty() {
        25.0
    } else {
        let avg_interval = intervals.iter().sum::<f64>() / intervals.len() as f64;
        1.0 / avg_interval
    }
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
        .map_err(|e| {
            eprintln!("---X get_flight_metrics: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;

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
    let end_ts = points.last().unwrap().ts;
    let t0 = start_ts;

    let mut samples = Vec::with_capacity(points.len());
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;

        let raw_roll = met::get_raw_roll(obj);
        let kalman_roll = met::get_kalman_roll(obj);
        let des_roll = met::get_any(obj, "phi_ref", &["DesiredAngleRoll"]);
        
        let raw_pitch = met::get_raw_pitch(obj);
        let kalman_pitch = met::get_kalman_pitch(obj);
        let des_pitch = met::get_any(obj, "theta_ref", &["DesiredAnglePitch"]);

        samples.push(met::AngleSample { 
            t_rel, 
            roll: raw_roll, 
            des_roll, 
            kalman_roll, 
            pitch: raw_pitch, 
            des_pitch, 
            kalman_pitch 
        });
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
            eprintln!("---X get_error_comparison: {e}");
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

        let raw_roll = met::get_raw_roll(obj);
        let kalman_roll = met::get_kalman_roll(obj);
        let des_roll = met::get_any(obj, "phi_ref", &["DesiredAngleRoll"]);

        samples.push(met::AngleSample {
            t_rel,
            roll: raw_roll,
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
            eprintln!("---X get_flight_metrics_full: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;

    if points.len() < 2 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data", fid)));
    }

    let start_ts = points.first().unwrap().ts;
    let end_ts = points.last().unwrap().ts;
    let t0 = start_ts;

    let mut samples = Vec::with_capacity(points.len());
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;

        let raw_roll = met::get_raw_roll(obj);
        let kalman_roll = met::get_kalman_roll(obj);
        let des_roll = met::get_any(obj, "phi_ref", &["DesiredAngleRoll"]);
        
        let raw_pitch = met::get_raw_pitch(obj);
        let kalman_pitch = met::get_kalman_pitch(obj);
        let des_pitch = met::get_any(obj, "theta_ref", &["DesiredAnglePitch"]);

        samples.push(met::AngleSample { 
            t_rel, 
            roll: raw_roll, 
            des_roll, 
            kalman_roll, 
            pitch: raw_pitch, 
            des_pitch, 
            kalman_pitch 
        });
    }

    let metrics = met::compute_full_flight_metrics(&fid, &samples, start_ts, end_ts);
    
    if metrics.sample_count > 50 && metrics.error_metrics.rmse_roll.is_some() {
        match store_flight_metrics(&ctx.questdb, &fid, &metrics).await {
            Ok(_) => println!("---! Vuelo {} guardado en histórico", fid),
            Err(e) => eprintln!("---X Error guardando en histórico: {e}"),
        }
    } else {
        println!("---! Vuelo {} no guardado en histórico (muestras insuficientes o datos inválidos)", fid);
    }
    
    Ok(Json(metrics))
}

#[axum::debug_handler]
pub async fn get_flight_spectrum(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<FlightSpectrum>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("---X get_flight_spectrum: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;
    
    if points.len() < 10 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data ({} points)", fid, points.len())));
    }
    
    // Calcular frecuencia de muestreo
    let sample_rate_hz = if points.len() > 1 {
        let mut intervals = Vec::new();
        for i in 1..points.len() {
            let dt = (points[i].ts - points[i-1].ts).num_milliseconds() as f64 / 1000.0;
            if dt > 0.0 && dt < 1.0 {
                intervals.push(dt);
            }
        }
        
        if intervals.is_empty() {
            25.0
        } else {
            let avg_interval = intervals.iter().sum::<f64>() / intervals.len() as f64;
            1.0 / avg_interval
        }
    } else {
        25.0
    };  
    
    // Extraer señales usando tus funciones
    let mut error_signal = Vec::new();
    let mut motor_signal = Vec::new();
    let mut acc_x_signal = Vec::new();
    let mut acc_y_signal = Vec::new();
    let mut acc_z_signal = Vec::new();
    
    for p in &points {
        let obj = &p.payload;
        
        // Error: phi_ref - KalmanAngleRoll
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let kalman_roll = met::get_kalman_roll(obj);
        
        if let (Some(phi), Some(kalman)) = (phi_ref, kalman_roll) {
            error_signal.push(phi - kalman);
        }
        
        // Motores
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
    
    let create_spectrum = |signal: &[f64], _name: &str| -> Spectrum {
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
    
    let error_spectrum = create_spectrum(&error_signal, "error");
    let motors_spectrum = create_spectrum(&motor_signal, "motors");
    let acc_x_spectrum = create_spectrum(&acc_x_signal, "acc_x");
    let acc_y_spectrum = create_spectrum(&acc_y_signal, "acc_y");
    let acc_z_spectrum = create_spectrum(&acc_z_signal, "acc_z");
    
    let mut correlations = Vec::new();
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

/// Obtiene el presupuesto de incertidumbre para un vuelo
#[axum::debug_handler]
pub async fn get_flight_uncertainty(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<UncertaintyResponse>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("---X get_flight_uncertainty: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;
    
    if points.len() < 10 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data", fid)));
    }
    
    let (raw_field, kalman_field) = identify_filtrado_vs_crudo(&points);
    
    // Extraer señales
    let mut errors = Vec::new();
    let mut raw_rolls = Vec::new();
    let mut motor_signals = Vec::new();
    
    for p in &points {
        let obj = &p.payload;
        
        // Error usando kalman detectado
        let phi_ref = met::get_any(obj, "phi_ref", &["DesiredAngleRoll"]);
        let kalman_roll = met::get_any(obj, &kalman_field, &[]);
        if let (Some(phi), Some(kalman)) = (phi_ref, kalman_roll) {
            errors.push(phi - kalman);
        }
        
        // Raw usando raw detectado
        if let Some(raw) = met::get_any(obj, &raw_field, &[]) {
            raw_rolls.push(raw);
        }
        
        // Motores
        let m1 = met::get_any(obj, "MotorInput1", &[]);
        let m2 = met::get_any(obj, "MotorInput2", &[]);
        let m3 = met::get_any(obj, "MotorInput3", &[]);
        let m4 = met::get_any(obj, "MotorInput4", &[]);
        let motor_vals: Vec<f64> = [m1, m2, m3, m4].iter().filter_map(|&x| x).collect();
        if !motor_vals.is_empty() {
            let avg = motor_vals.iter().sum::<f64>() / motor_vals.len() as f64;
            motor_signals.push(avg);
        }
    }
    
    if errors.is_empty() {
        println!("---! No se encontró error, usando valores por defecto");
        let default_error = 0.016;
        errors.push(default_error);
    }
    
    // 2. Calcular cada fuente de incertidumbre

    let imu_noise_std = if !raw_rolls.is_empty() {
        let mean = raw_rolls.iter().sum::<f64>() / raw_rolls.len() as f64;
        let variance = raw_rolls.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / raw_rolls.len() as f64;
        variance.sqrt()
    } else {
        0.5  // valor por defecto
    };
    
    // Fuente 2: Vibración (amplitud del error en la frecuencia dominante de motores)
    let vibration_amplitude = if motor_signals.len() > 10 && errors.len() > 10 {
        use crate::analysis::fft::compute_spectrum;
        
        // Calcular espectro del error
        let error_spectrum = compute_spectrum(&errors, 25.0, 5);
        
        // Calcular espectro de motores
        let motor_spectrum = compute_spectrum(&motor_signals, 25.0, 5);
        
        // Buscar frecuencias coincidentes
        let mut max_vibration: f64 = 0.1;  // valor por defecto
        
        for (freq_motor, _mag_motor) in &motor_spectrum.dominant_peaks {
            for (freq_error, mag_error) in &error_spectrum.dominant_peaks {
                if (freq_error - freq_motor).abs() < 0.5 {
                    max_vibration = max_vibration.max(*mag_error);
                }
            }
        }
        
        max_vibration
    } else {
        0.5
    };
    
    // Fuente 3: Error residual del Kalman (RMS del error observado)
    let kalman_residual = (errors.iter().map(|e| e * e).sum::<f64>() / errors.len() as f64).sqrt();
    
    // Fuente 4: Jitter de temporización (asumimos pequeño por ahora)
    let timing_jitter = 0.05;  // 0.05 grados por jitter
    
    // Fuente 5: Bias drift (asumimos pequeño)
    let bias_drift = 0.1;
    
    // 3. Crear el presupuesto de incertidumbre
    let sources = vec![
        UncertaintySource {
            name: "Ruido IMU".to_string(),
            value: imu_noise_std,
            distribution: DistributionType::Normal { mean: 0.0, std_dev: imu_noise_std },
            description: "Ruido de alta frecuencia del sensor IMU medido en reposo".to_string(),
        },
        UncertaintySource {
            name: "Vibración".to_string(),
            value: vibration_amplitude,
            distribution: DistributionType::Uniform { min: -vibration_amplitude, max: vibration_amplitude },
            description: "Error inducido por vibraciones de motores".to_string(),
        },
        UncertaintySource {
            name: "Residual Kalman".to_string(),
            value: kalman_residual,
            distribution: DistributionType::Normal { mean: 0.0, std_dev: kalman_residual },
            description: "Error residual después del filtro Kalman".to_string(),
        },
        UncertaintySource {
            name: "Jitter temporal".to_string(),
            value: timing_jitter,
            distribution: DistributionType::Uniform { min: -timing_jitter, max: timing_jitter },
            description: "Variación en el tiempo de recepción de paquetes".to_string(),
        },
        UncertaintySource {
            name: "Bias drift".to_string(),
            value: bias_drift,
            distribution: DistributionType::Normal { mean: 0.0, std_dev: bias_drift },
            description: "Deriva lenta del bias del giroscopio".to_string(),
        },
    ];
    
    let budget = create_uncertainty_budget(sources);
    
    // 4. Simulación Monte Carlo
    let monte_carlo = monte_carlo_simulation(&budget.sources, 10000);
    
    // 5. Validación: verificar si el error observado está dentro del intervalo
    let observed_error_rms = kalman_residual;
    let interval_lower = -budget.expanded_uncertainty_k2;
    let interval_upper = budget.expanded_uncertainty_k2;
    let within_interval = observed_error_rms <= budget.expanded_uncertainty_k2;
    
    let validation = ValidationResult {
        observed_error_rms,
        within_interval,
        interval_lower,
        interval_upper,
    };
    
    let response = UncertaintyResponse {
        flight_id: fid,
        budget,
        monte_carlo,
        validation,
    };
    
    Ok(Json(response))
}

/// Para cada vuelo, analizamos qué campos tienen menos ruido
fn identify_filtrado_vs_crudo(points: &[FlightPoint]) -> (String, String) {
    let mut raw_candidates = Vec::new();
    let mut kalman_candidates = Vec::new();
    
    // Tomar más muestras para mejor estimación
    let sample_size = points.len().min(500);
    
    // Lista de posibles campos de ángulo
    let angle_fields = [
        "AngleRoll", "AngleRoll_est", "KalmanAngleRoll",
        "roll", "Roll", "AngleRoll_raw", "filtered_roll"
    ];
    
    // Calcular varianza de cada campo
    for field in angle_fields {
        let values: Vec<f64> = points[..sample_size].iter()
            .filter_map(|p| {
                p.payload.get(field)
                    .and_then(|v| v.as_f64())
            })
            .collect();
        
        if values.len() > 10 {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            let variance = values.iter()
                .map(|x| (x - mean).powi(2))
                .sum::<f64>() / values.len() as f64;
            
            // Clasificar por nombre
            if field.contains("est") || field.contains("raw") {
                raw_candidates.push((field, variance));
            } else {
                kalman_candidates.push((field, variance));
            }
        }
    }
    
    // El de menor varianza es el filtrado
    let kalman = if !kalman_candidates.is_empty() {
        kalman_candidates.iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|&(name, _)| {
                name.to_string()
            })
            .unwrap_or_else(|| "AngleRoll".to_string())
    } else {
        "AngleRoll".to_string()
    };
    
    // El de mayor varianza es el crudo
    let raw = if !raw_candidates.is_empty() {
        raw_candidates.iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|&(name, _)| {
                name.to_string()
            })
            .unwrap_or_else(|| "AngleRoll_est".to_string())
    } else {
        "AngleRoll_est".to_string()
    };
    
    (raw, kalman)
}

/// Obtiene el análisis de anomalías para un vuelo
#[axum::debug_handler]
pub async fn get_flight_anomalies(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<AnomalyResponse>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    
    // Obtener puntos del vuelo
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("---X get_flight_anomalies: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;
    
    if points.len() < 10 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data", fid)));
    }
    
    let t0 = points[0].ts;
    
    // Extraer señales
    let mut roll_errors = Vec::new();
    let mut pitch_errors = Vec::new();
    let mut raw_roll = Vec::new();
    let mut raw_pitch = Vec::new();
    
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;
        
        // Error roll
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let kalman_roll = met::get_kalman_roll(obj);
        if let (Some(phi), Some(kalman)) = (phi_ref, kalman_roll) {
            roll_errors.push((t_rel, phi - kalman));
        }
        
        // Error pitch
        let theta_ref = met::get_any(obj, "theta_ref", &[]);
        let kalman_pitch = met::get_kalman_pitch(obj);
        if let (Some(theta), Some(kalman)) = (theta_ref, kalman_pitch) {
            pitch_errors.push((t_rel, theta - kalman));
        }
        
        // Raw roll
        if let Some(raw) = met::get_any(obj, "AngleRoll", &["AngleRoll_est"]) {
            raw_roll.push((t_rel, raw));
        }
        
        // Raw pitch
        if let Some(raw) = met::get_any(obj, "AnglePitch", &["AnglePitch_est"]) {
            raw_pitch.push((t_rel, raw));
        }
    }
    
    // Analizar anomalías
    let report = analyze_flight_anomalies(
        &fid,
        &roll_errors,
        &pitch_errors,
        &raw_roll,
        &raw_pitch,
    );
    
    let response = AnomalyResponse {
        flight_id: fid,
        report,
    };
    
    Ok(Json(response))
}

/// Obtiene la matriz de correlaciones para un vuelo
#[axum::debug_handler]
pub async fn get_flight_correlations(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<CorrelationResponse>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    
    // Obtener puntos del vuelo
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("---X get_flight_correlations: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;
    
    if points.len() < 10 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data", fid)));
    }
    
    let t0 = points[0].ts;
    
    // Extraer muestras
    let mut samples = Vec::with_capacity(points.len());
    
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;
        
        let raw_roll = met::get_raw_roll(obj);
        let raw_pitch = met::get_raw_pitch(obj);
        let kalman_roll = met::get_kalman_roll(obj);
        let kalman_pitch = met::get_kalman_pitch(obj);
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let theta_ref = met::get_any(obj, "theta_ref", &[]);
        
        samples.push(AngleSample {
            t_rel,
            roll: raw_roll,
            des_roll: phi_ref,
            kalman_roll,
            pitch: raw_pitch,
            des_pitch: theta_ref,
            kalman_pitch,
        });
    }
    
    let report = analyze_correlations(&fid, &samples);
    
    let response = CorrelationResponse {
        flight_id: fid,
        report,
    };
    
    Ok(Json(response))
}

#[axum::debug_handler]
pub async fn get_flight_trend(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<TrendResponse>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    
    // Obtener puntos del vuelo
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("---X get_flight_trend: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;
    
    if points.len() < 20 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data (min 20 points)", fid)));
    }
    
    let t0 = points[0].ts;
    
    // Extraer errores de seguimiento
    let mut roll_errors = Vec::new();   // (timestamp, error)
    let mut pitch_errors = Vec::new();
    
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;
        
        // Error roll
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let kalman_roll = met::get_kalman_roll(obj);
        if let (Some(phi), Some(kalman)) = (phi_ref, kalman_roll) {
            roll_errors.push((t_rel, phi - kalman));
        }
        
        // Error pitch
        let theta_ref = met::get_any(obj, "theta_ref", &[]);
        let kalman_pitch = met::get_kalman_pitch(obj);
        if let (Some(theta), Some(kalman)) = (theta_ref, kalman_pitch) {
            pitch_errors.push((t_rel, theta - kalman));
        }
    }
    
    // Analizar tendencias (predecir los próximos 2 segundos)
    let seconds_ahead = 2.0;
    let report = analyze_trends(&fid, &roll_errors, &pitch_errors, seconds_ahead);
    
    let response = TrendResponse {
        flight_id: fid,
        report,
    };
    
    Ok(Json(response))
}

#[axum::debug_handler]
pub async fn get_flight_recommendations(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<RecommendationsResponse>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    
    // Obtener puntos del vuelo
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("---X get_flight_recommendations: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;
    
    if points.len() < 10 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data", fid)));
    }
    
    // Obtener métricas (usando tu infraestructura existente)
    let start_ts = points.first().unwrap().ts;
    let end_ts = points.last().unwrap().ts;
    let t0 = start_ts;
    
    let mut samples = Vec::with_capacity(points.len());
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;

        let raw_roll = met::get_raw_roll(obj);
        let raw_pitch = met::get_raw_pitch(obj);
        let kalman_roll = met::get_kalman_roll(obj);
        let kalman_pitch = met::get_kalman_pitch(obj);
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let theta_ref = met::get_any(obj, "theta_ref", &[]);
        
        samples.push(met::AngleSample {
            t_rel,
            roll: raw_roll,
            des_roll: phi_ref,
            kalman_roll,
            pitch: raw_pitch,
            des_pitch: theta_ref,
            kalman_pitch,
        });
    }
    
    let metrics = met::compute_full_flight_metrics(&fid, &samples, start_ts, end_ts);
    
    // Obtener espectro (usando tus funciones)
    let sample_rate_hz = calculate_sample_rate(&points);
    let spectrum = get_flight_spectrum_data_with_functions(&points, sample_rate_hz);
    
    // Obtener anomalías (usando tus funciones)
    let mut roll_errors = Vec::new();
    let mut pitch_errors = Vec::new();
    let mut raw_roll_signal = Vec::new();
    let mut raw_pitch_signal = Vec::new();
    
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;
        
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let kalman_roll = met::get_kalman_roll(obj);
        if let (Some(phi), Some(kalman)) = (phi_ref, kalman_roll) {
            roll_errors.push((t_rel, phi - kalman));
        }
        
        let theta_ref = met::get_any(obj, "theta_ref", &[]);
        let kalman_pitch = met::get_kalman_pitch(obj);
        if let (Some(theta), Some(kalman)) = (theta_ref, kalman_pitch) {
            pitch_errors.push((t_rel, theta - kalman));
        }
        
        if let Some(raw) = met::get_raw_roll(obj) {
            raw_roll_signal.push((t_rel, raw));
        }
        
        if let Some(raw) = met::get_raw_pitch(obj) {
            raw_pitch_signal.push((t_rel, raw));
        }
    }
    
    let anomalies = analyze_flight_anomalies(&fid, &roll_errors, &pitch_errors, &raw_roll_signal, &raw_pitch_signal);
    
    // Generar recomendaciones
    let report = generate_recommendations(
        &fid,
        match metrics.flight_type {
            met::FlightType::Reposo => "reposo",
            met::FlightType::Hover => "hover",
            met::FlightType::Maniobra => "maniobra",
            met::FlightType::Desconocido => "desconocido",
        },
        &metrics,
        &spectrum,
        &anomalies,
    );
    
    let response = RecommendationsResponse {
        flight_id: fid,
        report,
    };
    
    Ok(Json(response))
}

// Función auxiliar para obtener datos de espectro usando tus funciones
fn get_flight_spectrum_data_with_functions(
    points: &[crate::ws_server::questdb::FlightPoint], 
    sample_rate_hz: f64
) -> crate::config::spectrum_types::FlightSpectrum {
    use crate::analysis::fft::compute_spectrum;
    use crate::config::spectrum_types::{FlightSpectrum, Spectrum, Peak, Correlation};
    
    let mut error_signal = Vec::new();
    let mut motor_signal = Vec::new();
    let mut acc_x_signal = Vec::new();
    let mut acc_y_signal = Vec::new();
    let mut acc_z_signal = Vec::new();
    
    for p in points {
        let obj = &p.payload;
        
        // Error
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let kalman_roll = met::get_kalman_roll(obj);
        if let (Some(phi), Some(kalman)) = (phi_ref, kalman_roll) {
            error_signal.push(phi - kalman);
        }
        
        // Motores
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
    
    // Espectro de error
    let fft_error = compute_spectrum(&error_signal, sample_rate_hz, 5);
    let error_spectrum = Spectrum {
        frequencies_hz: fft_error.frequencies_hz,
        magnitudes: fft_error.magnitudes,
        dominant_peaks: fft_error.dominant_peaks.iter()
            .map(|(freq, mag)| Peak { frequency_hz: *freq, magnitude: *mag })
            .collect(),
    };
    
    // Espectro de motores
    let fft_motor = compute_spectrum(&motor_signal, sample_rate_hz, 5);
    let motors_spectrum = Spectrum {
        frequencies_hz: fft_motor.frequencies_hz,
        magnitudes: fft_motor.magnitudes,
        dominant_peaks: fft_motor.dominant_peaks.iter()
            .map(|(freq, mag)| Peak { frequency_hz: *freq, magnitude: *mag })
            .collect(),
    };
    
    // Espectros de acelerómetros
    let fft_acc_x = compute_spectrum(&acc_x_signal, sample_rate_hz, 5);
    let acc_x_spectrum = Spectrum {
        frequencies_hz: fft_acc_x.frequencies_hz,
        magnitudes: fft_acc_x.magnitudes,
        dominant_peaks: fft_acc_x.dominant_peaks.iter()
            .map(|(freq, mag)| Peak { frequency_hz: *freq, magnitude: *mag })
            .collect(),
    };
    
    let fft_acc_y = compute_spectrum(&acc_y_signal, sample_rate_hz, 5);
    let acc_y_spectrum = Spectrum {
        frequencies_hz: fft_acc_y.frequencies_hz,
        magnitudes: fft_acc_y.magnitudes,
        dominant_peaks: fft_acc_y.dominant_peaks.iter()
            .map(|(freq, mag)| Peak { frequency_hz: *freq, magnitude: *mag })
            .collect(),
    };
    
    let fft_acc_z = compute_spectrum(&acc_z_signal, sample_rate_hz, 5);
    let acc_z_spectrum = Spectrum {
        frequencies_hz: fft_acc_z.frequencies_hz,
        magnitudes: fft_acc_z.magnitudes,
        dominant_peaks: fft_acc_z.dominant_peaks.iter()
            .map(|(freq, mag)| Peak { frequency_hz: *freq, magnitude: *mag })
            .collect(),
    };
    
    // Correlaciones
    let mut correlations = Vec::new();
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
    
    FlightSpectrum {
        flight_id: String::new(),
        sample_rate_hz,
        sample_count: points.len(),
        error_spectrum,
        motors_spectrum,
        acc_x_spectrum,
        acc_y_spectrum,
        acc_z_spectrum,
        correlations,
    }
}

#[axum::debug_handler]
pub async fn get_flight_score(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<ScoreResponse>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    
    // Obtener puntos del vuelo
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("---X get_flight_score: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;
    
    if points.len() < 10 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data", fid)));
    }
    
    // Obtener métricas completas
    let start_ts = points.first().unwrap().ts;
    let end_ts = points.last().unwrap().ts;
    let t0 = start_ts;
    
    let mut samples = Vec::with_capacity(points.len());
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;
        
        let raw_roll = met::get_raw_roll(obj);
        let raw_pitch = met::get_raw_pitch(obj);
        let kalman_roll = met::get_kalman_roll(obj);
        let kalman_pitch = met::get_kalman_pitch(obj);
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let theta_ref = met::get_any(obj, "theta_ref", &[]);
        
        samples.push(met::AngleSample {
            t_rel,
            roll: raw_roll,
            des_roll: phi_ref,
            kalman_roll,
            pitch: raw_pitch,
            des_pitch: theta_ref,
            kalman_pitch,
        });
    }
    
    let metrics = met::compute_full_flight_metrics(&fid, &samples, start_ts, end_ts);
    
    // Obtener espectro
    let sample_rate_hz = calculate_sample_rate(&points);
    let spectrum = get_flight_spectrum_data_with_functions(&points, sample_rate_hz);
    
    // Obtener anomalías
    let mut roll_errors = Vec::new();
    let mut pitch_errors = Vec::new();
    let mut raw_roll_signal = Vec::new();
    let mut raw_pitch_signal = Vec::new();
    
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;
        
        let phi_ref = met::get_any(obj, "phi_ref", &[]);
        let kalman_roll = met::get_kalman_roll(obj);
        if let (Some(phi), Some(kalman)) = (phi_ref, kalman_roll) {
            roll_errors.push((t_rel, phi - kalman));
        }
        
        let theta_ref = met::get_any(obj, "theta_ref", &[]);
        let kalman_pitch = met::get_kalman_pitch(obj);
        if let (Some(theta), Some(kalman)) = (theta_ref, kalman_pitch) {
            pitch_errors.push((t_rel, theta - kalman));
        }
        
        if let Some(raw) = met::get_raw_roll(obj) {
            raw_roll_signal.push((t_rel, raw));
        }
        
        if let Some(raw) = met::get_raw_pitch(obj) {
            raw_pitch_signal.push((t_rel, raw));
        }
    }
    
    let anomalies = analyze_flight_anomalies(&fid, &roll_errors, &pitch_errors, &raw_roll_signal, &raw_pitch_signal);
    
    // Calcular score
    let score = compute_quality_score(&metrics, &spectrum, &anomalies);
    
    let response = ScoreResponse {
        flight_id: fid,
        score,
    };
    
    Ok(Json(response))
}

#[axum::debug_handler]
pub async fn get_flight_historical_comparison(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<HistoricalComparison>, ApiError> {
    let ctx = state.ws_ctx.lock().await;
    
    // Obtener métricas completas del vuelo actual
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("---X get_flight_historical_comparison: {e}");
            ApiError::Internal("Failed to fetch flight points".to_string())
        })?;
    
    if points.len() < 10 {
        return Err(ApiError::NotFound(format!("Flight {} has insufficient data", fid)));
    }
    
    let start_ts = points.first().unwrap().ts;
    let end_ts = points.last().unwrap().ts;
    let t0 = start_ts;
    
    let mut samples = Vec::with_capacity(points.len());
    for p in &points {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;
        
        let raw_roll = met::get_raw_roll(obj);
        let kalman_roll = met::get_kalman_roll(obj);
        let des_roll = met::get_any(obj, "phi_ref", &["DesiredAngleRoll"]);
        
        let raw_pitch = met::get_raw_pitch(obj);
        let kalman_pitch = met::get_kalman_pitch(obj);
        let des_pitch = met::get_any(obj, "theta_ref", &["DesiredAnglePitch"]);
        
        samples.push(met::AngleSample {
            t_rel,
            roll: raw_roll,
            des_roll,
            kalman_roll,
            pitch: raw_pitch,
            des_pitch,
            kalman_pitch,
        });
    }
    
    let current_metrics = met::compute_full_flight_metrics(&fid, &samples, start_ts, end_ts);
    
    // Guardar métricas en histórico (para futuras comparaciones)
    if let Err(e) = store_flight_metrics(&ctx.questdb, &fid, &current_metrics).await {
        eprintln!("⚠️ No se pudo guardar métricas históricas: {e}");
    }
    
    // Obtener métricas históricas del mismo tipo
    let flight_type_str = match current_metrics.flight_type {
        met::FlightType::Reposo => "reposo",
        met::FlightType::Hover => "hover",
        met::FlightType::Maniobra => "maniobra",
        met::FlightType::Desconocido => "desconocido",
    };
    
    let historical_metrics = get_historical_metrics(&ctx.questdb, flight_type_str)
        .await
        .map_err(|e| {
            eprintln!("---X get_historical_metrics: {e}");
            ApiError::Internal("Failed to fetch historical metrics".to_string())
        })?;
    
    let comparison = compare_flight_with_historical(&current_metrics, &historical_metrics);
    
    Ok(Json(comparison))
}