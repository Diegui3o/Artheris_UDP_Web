use serde::Serialize;
use serde_json::Value;

pub const FIELD_ROLL: &str = "AngleRoll";
pub const FIELD_PITCH: &str = "AnglePitch";
pub const FIELD_DES_ROLL: &str = "DesiredAngleRoll";
pub const FIELD_DES_PITCH: &str = "DesiredAnglePitch";

pub const ALT_ROLL: &[&str] = &["AngleRoll_est","roll","Roll"];
pub const ALT_PITCH: &[&str] = &["AnglePitch_est","pitch","Pitch"];
pub const ALT_DES_ROLL: &[&str] = &["des_roll","roll_setpoint","target_roll","phi_ref"];
pub const ALT_DES_PITCH: &[&str] = &["des_pitch","pitch_setpoint","target_pitch","theta_ref"];

trait ValueExt {
    fn type_str(&self) -> &'static str;
}

impl ValueExt for Value {
    fn type_str(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}
fn read_num(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_i64().map(|x| x as f64))
        .or_else(|| v.as_u64().map(|x| x as f64))
        .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
}

fn get_one(obj: &Value, key: &str) -> Option<f64> {
    obj.get(key).and_then(read_num)
        .or_else(|| obj.get("values").and_then(|v| v.get(key)).and_then(read_num))
}

pub fn get_any(obj: &Value, primary: &str, alts: &[&str]) -> Option<f64> {
    get_one(obj, primary)
        .or_else(|| alts.iter().find_map(|&k| get_one(obj, k)))
}

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
    pub kalman_roll: Option<f64>,
    pub pitch: Option<f64>,
    pub des_pitch: Option<f64>,
    pub kalman_pitch: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AngleMetrics {
    pub rmse_roll: Option<f64>,
    pub rmse_pitch: Option<f64>,
    pub itae_roll: Option<f64>,
    pub itae_pitch: Option<f64>,
    pub mae_roll: Option<f64>,
    pub mae_pitch: Option<f64>,
    pub variance_roll: Option<f64>,
    pub variance_pitch: Option<f64>,
    pub std_dev_roll: Option<f64>,
    pub std_dev_pitch: Option<f64>,
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

#[derive(Debug, Clone, Serialize)]
pub struct ErrorComparisonMetrics {
    pub raw_rmse: Option<f64>,
    pub raw_variance: Option<f64>,
    pub kalman_rmse: Option<f64>,
    pub kalman_variance: Option<f64>,
    pub improvement_rmse_percent: Option<f64>,
    pub improvement_variance_percent: Option<f64>,
}

pub fn detect_fields(obj: &Value) -> (Option<&str>, Option<&str>) {
    // Lista de posibles nombres para campo crudo (ruidoso)
    let raw_candidates = &[
        "AngleRoll_est",    // tu nombre
        "AngleRoll_raw",    // alternativo
        "roll_raw",         // otro
        "raw_roll",         // otro
        "AngleRoll",        // fallback (si no hay otro)
    ];
    
    // Lista de posibles nombres para campo filtrado (suave)
    let kalman_candidates = &[
        "AngleRoll",        // tu filtrado
        "KalmanAngleRoll",  // nombre estándar
        "filtered_roll",    // alternativo
        "roll_filtered",    // otro
        "roll_est",         // otro
    ];
    
    let obj_map = match obj.as_object() {
        Some(m) => m,
        None => return (None, None),
    };
    
    // Buscar crudo
    let raw_field = raw_candidates.iter()
        .find(|&&name| obj_map.contains_key(name))
        .map(|&name| name);
    
    // Buscar filtrado
    let kalman_field = kalman_candidates.iter()
        .find(|&&name| obj_map.contains_key(name))
        .map(|&name| name);
    
    // Si no se encontró filtrado pero hay AngleRoll, asumimos que es filtrado
    if kalman_field.is_none() && obj_map.contains_key("AngleRoll") {
        return (raw_field, Some("AngleRoll"));
    }
    
    (raw_field, kalman_field)
}

pub fn compute_angle_metrics(samples: &[AngleSample]) -> AngleMetrics {
    
    if samples.len() < 2 {
        println!("Not enough samples ({} < 2)", samples.len());
        return AngleMetrics { 
            rmse_roll: None, rmse_pitch: None, 
            itae_roll: None, itae_pitch: None,
            mae_roll: None, mae_pitch: None,
            variance_roll: None, variance_pitch: None,
            std_dev_roll: None, std_dev_pitch: None,
            n_segments_used: 0, duration_sec: 0.0 
        };
    }

    let duration_sec = samples.last().unwrap().t_rel - samples.first().unwrap().t_rel;
    
    if duration_sec <= 0.0 {
        println!("Invalid duration: {}", duration_sec);
        return AngleMetrics { 
            rmse_roll: None, rmse_pitch: None, 
            itae_roll: None, itae_pitch: None,
            mae_roll: None, mae_pitch: None,
            variance_roll: None, variance_pitch: None,
            std_dev_roll: None, std_dev_pitch: None,
            n_segments_used: 0, duration_sec: 0.0 
        };
    }

    let mut sum_abs_roll_dt = 0.0;
    let mut sum_sq_roll_dt = 0.0;
    let mut sum_itae_roll = 0.0;
    let mut _sum_error_roll = 0.0;
    let mut roll_dt_total = 0.0;
    let mut roll_used = 0usize;
    let mut roll_errors = Vec::new(); 

    let mut sum_abs_pitch_dt = 0.0;
    let mut sum_sq_pitch_dt = 0.0;
    let mut sum_itae_pitch = 0.0;
    let mut _sum_error_pitch = 0.0;
    let mut pitch_dt_total = 0.0;
    let mut pitch_used = 0usize;
    let mut pitch_errors = Vec::new();

    for w in samples.windows(2) {
        let a = &w[0];
        let b = &w[1];
        let dt = (b.t_rel - a.t_rel).max(0.0);
        if dt <= 0.0 { continue; }

        // Roll
        if let (Some(r_a), Some(dr_a)) = (a.roll, a.des_roll) {
            let e = r_a - dr_a;
            sum_abs_roll_dt += e.abs() * dt;
            sum_sq_roll_dt += e * e * dt;
            sum_itae_roll += a.t_rel * e.abs() * dt;
            _sum_error_roll += e * dt;
            roll_dt_total += dt;
            roll_used += 1;
            roll_errors.push(e);
        }

        // Pitch
        if let (Some(p_a), Some(dp_a)) = (a.pitch, a.des_pitch) {
            let e = p_a - dp_a;
            sum_abs_pitch_dt += e.abs() * dt;
            sum_sq_pitch_dt += e * e * dt;
            sum_itae_pitch += a.t_rel * e.abs() * dt;
            _sum_error_pitch += e * dt;
            pitch_dt_total += dt;
            pitch_used += 1;
            pitch_errors.push(e);
        }
    }

    // Calcular métricas existentes
    let mae_roll = if roll_dt_total > 0.0 { Some(sum_abs_roll_dt / roll_dt_total) } else { None };
    let rmse_roll = if roll_dt_total > 0.0 { Some((sum_sq_roll_dt / roll_dt_total).sqrt()) } else { None };
    let itae_roll = if roll_used > 0 { Some(sum_itae_roll) } else { None };
    
    let mae_pitch = if pitch_dt_total > 0.0 { Some(sum_abs_pitch_dt / pitch_dt_total) } else { None };
    let rmse_pitch = if pitch_dt_total > 0.0 { Some((sum_sq_pitch_dt / pitch_dt_total).sqrt()) } else { None };
    let itae_pitch = if pitch_used > 0 { Some(sum_itae_pitch) } else { None };
    
    // ⭐ NUEVO: Calcular media, varianza y desviación estándar
    let (_mean_roll, variance_roll, std_dev_roll) = if !roll_errors.is_empty() {
        let mean = roll_errors.iter().sum::<f64>() / roll_errors.len() as f64;
        let variance = roll_errors.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / roll_errors.len() as f64;
        let std_dev = variance.sqrt();
        (Some(mean), Some(variance), Some(std_dev))
    } else {
        (None, None, None)
    };
    
    let (_mean_pitch, variance_pitch, std_dev_pitch) = if !pitch_errors.is_empty() {
        let mean = pitch_errors.iter().sum::<f64>() / pitch_errors.len() as f64;
        let variance = pitch_errors.iter().map(|e| (e - mean).powi(2)).sum::<f64>() / pitch_errors.len() as f64;
        let std_dev = variance.sqrt();
        (Some(mean), Some(variance), Some(std_dev))
    } else {
        (None, None, None)
    };

    AngleMetrics {
        rmse_roll, rmse_pitch,
        itae_roll, itae_pitch,
        mae_roll, mae_pitch,
        variance_roll, variance_pitch,
        std_dev_roll, std_dev_pitch,
        n_segments_used: roll_used + pitch_used,
        duration_sec,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ComparisonMetrics {
    pub raw_rms: Option<f64>,
    pub kalman_rms: Option<f64>,
    pub improvement_percent: Option<f64>,
    pub sample_count: usize,
    pub duration_sec: f64,
}

pub fn compute_comparison_metrics(
    samples: &[AngleSample],
    use_roll: bool,
) -> ComparisonMetrics {
    if samples.len() < 2 {
        return ComparisonMetrics {
            raw_rms: None,
            kalman_rms: None,
            improvement_percent: None,
            sample_count: 0,
            duration_sec: 0.0,
        };
    }

    let duration_sec = samples.last().unwrap().t_rel - samples.first().unwrap().t_rel;
    if duration_sec <= 0.0 {
        return ComparisonMetrics {
            raw_rms: None,
            kalman_rms: None,
            improvement_percent: None,
            sample_count: 0,
            duration_sec: 0.0,
        };
    }

    let mut sum_raw_sq = 0.0;
    let mut sum_kalman_sq = 0.0;
    let mut count = 0;

    for sample in samples {
        let (raw, kalman) = if use_roll {
            (sample.roll, sample.kalman_roll)
        } else {
            (sample.pitch, sample.kalman_pitch)
        };
        
        if let (Some(r), Some(k)) = (raw, kalman) {
            sum_raw_sq += r * r;
            sum_kalman_sq += k * k;
            count += 1;
        }
    }

    let raw_rms = if count > 0 {
        Some((sum_raw_sq / count as f64).sqrt())
    } else {
        None
    };

    let kalman_rms = if count > 0 {
        Some((sum_kalman_sq / count as f64).sqrt())
    } else {
        None
    };

    let improvement_percent = match (raw_rms, kalman_rms) {
        (Some(raw), Some(kalman)) if raw > 0.0 => {
            Some(((raw - kalman) / raw) * 100.0)
        }
        _ => None,
    };

    ComparisonMetrics {
        raw_rms,
        kalman_rms,
        improvement_percent,
        sample_count: count,
        duration_sec,
    }
}

/// Métricas completas del vuelo (error + comparación)
#[derive(Debug, Clone, Serialize)]
pub struct FullFlightMetrics {
    pub flight_id: String,
    pub start_ts: String,
    pub end_ts: String,
    pub duration_sec: f64,
    pub sample_count: usize,
    pub error_metrics: AngleMetrics,
    pub comparison_roll: ComparisonMetrics,
    pub comparison_pitch: ComparisonMetrics,
}

/// Calcula todas las métricas para un vuelo (error + raw vs kalman)
pub fn compute_full_flight_metrics(
    flight_id: &str,
    samples: &[AngleSample],
    start_ts: chrono::DateTime<chrono::Utc>,
    end_ts: chrono::DateTime<chrono::Utc>,
) -> FullFlightMetrics {
    let duration_sec = (end_ts - start_ts).num_milliseconds() as f64 / 1000.0;
    
    let error_metrics = compute_angle_metrics(samples);
    let comparison_roll = compute_comparison_metrics(samples, true);
    let comparison_pitch = compute_comparison_metrics(samples, false);
    
    FullFlightMetrics {
        flight_id: flight_id.to_string(),
        start_ts: start_ts.to_rfc3339(),
        end_ts: end_ts.to_rfc3339(),
        duration_sec,
        sample_count: samples.len(),
        error_metrics,
        comparison_roll,
        comparison_pitch,
    }
}

pub fn compute_error_comparison(samples: &[AngleSample]) -> ErrorComparisonMetrics {
    if samples.len() < 2 {
        return ErrorComparisonMetrics {
            raw_rmse: None,
            raw_variance: None,
            kalman_rmse: None,
            kalman_variance: None,
            improvement_rmse_percent: None,
            improvement_variance_percent: None,
        };
    }

    let mut sum_raw_error_sq = 0.0;
    let mut sum_kalman_error_sq = 0.0;
    let mut sum_raw_error = 0.0;
    let mut sum_kalman_error = 0.0;
    let mut count = 0;

    for sample in samples {
        // error_raw = referencia - raw
        let error_raw = if let (Some(ref_val), Some(raw_val)) = (sample.des_roll, sample.roll) {
            Some(ref_val - raw_val)
        } else {
            None
        };
        
        // error_kalman = referencia - kalman
        let error_kalman = if let (Some(ref_val), Some(kalman_val)) = (sample.des_roll, sample.kalman_roll) {
            Some(ref_val - kalman_val)
        } else {
            None
        };
        
        if let (Some(e_raw), Some(e_kalman)) = (error_raw, error_kalman) {
            sum_raw_error_sq += e_raw * e_raw;
            sum_kalman_error_sq += e_kalman * e_kalman;
            sum_raw_error += e_raw;
            sum_kalman_error += e_kalman;
            count += 1;
        }
    }

    if count == 0 {
        return ErrorComparisonMetrics {
            raw_rmse: None,
            raw_variance: None,
            kalman_rmse: None,
            kalman_variance: None,
            improvement_rmse_percent: None,
            improvement_variance_percent: None,
        };
    }

    // Calcular RMSE
    let raw_rmse = Some((sum_raw_error_sq / count as f64).sqrt());
    let kalman_rmse = Some((sum_kalman_error_sq / count as f64).sqrt());
    
    // Calcular varianza
    let raw_mean = sum_raw_error / count as f64;
    let kalman_mean = sum_kalman_error / count as f64;
    
    let raw_variance = Some((sum_raw_error_sq / count as f64) - (raw_mean * raw_mean));
    let kalman_variance = Some((sum_kalman_error_sq / count as f64) - (kalman_mean * kalman_mean));
    
    // Calcular mejora porcentual
    let improvement_rmse_percent = match (raw_rmse, kalman_rmse) {
        (Some(raw), Some(kalman)) if raw > 0.0 => Some(((raw - kalman) / raw) * 100.0),
        _ => None,
    };
    
    let improvement_variance_percent = match (raw_variance, kalman_variance) {
        (Some(raw), Some(kalman)) if raw > 0.0 => Some(((raw - kalman) / raw) * 100.0),
        _ => None,
    };

    ErrorComparisonMetrics {
        raw_rmse,
        raw_variance,
        kalman_rmse,
        kalman_variance,
        improvement_rmse_percent,
        improvement_variance_percent,
    }
}