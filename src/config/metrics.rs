use serde::Serialize;
use std::sync::Arc;
use axum::{extract::{Path, State}, Json};
use serde_json::Value;

pub const FIELD_ROLL: &str = "AngleRoll";
pub const FIELD_PITCH: &str = "AnglePitch";
pub const FIELD_DES_ROLL: &str = "DesiredAngleRoll";
pub const FIELD_DES_PITCH: &str = "DesiredAnglePitch";

pub const ALT_ROLL: &[&str] = &["AngleRoll_est","roll","Roll"];
pub const ALT_PITCH: &[&str] = &["AnglePitch_est","pitch","Pitch"];
pub const ALT_DES_ROLL: &[&str] = &["des_roll","roll_setpoint","target_roll"];
pub const ALT_DES_PITCH: &[&str] = &["des_pitch","pitch_setpoint","target_pitch"];

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
fn get_f64_from(obj: &Value, k: &str) -> Option<f64> {
    // Helper to extract number from a Value
    let read_num = |v: &Value| {
        v.as_f64()
            .or_else(|| v.as_i64().map(|x| x as f64))
            .or_else(|| v.as_u64().map(|x| x as f64))
    };

    if let Some(v) = obj.get(k).and_then(&read_num) {
        return Some(v);
    }
    
    if let Some(values) = obj.get("values") {

        if let Some(v) = values.get(k).and_then(&read_num) {
            return Some(v);
        }
        if let Some(obj_map) = values.as_object() {
            for (key, val) in obj_map {
                if key.eq_ignore_ascii_case(k) {
                    if let Some(v) = read_num(val) {
                        return Some(v);
                    }
                }
            }
        }
    }
    
    // 3) Try alternative field names
    let alt_names = match k {
        FIELD_ROLL => ALT_ROLL,
        FIELD_PITCH => ALT_PITCH,
        FIELD_DES_ROLL => ALT_DES_ROLL,
        FIELD_DES_PITCH => ALT_DES_PITCH,
        _ => &[]
    };
    
    for &alt in alt_names {
        if alt != k {
            if let Some(v) = obj.get(alt).and_then(&read_num) {
                return Some(v);
            }

            if let Some(values) = obj.get("values") {
                // Try exact match first
                if let Some(v) = values.get(alt).and_then(&read_num) {
                    return Some(v);
                }

                if let Some(obj_map) = values.as_object() {
                    for (key, val) in obj_map {
                        if key.eq_ignore_ascii_case(alt) {
                            if let Some(v) = read_num(val) {
                                return Some(v);
                            }
                        }
                    }
                }
            }
        }
    }

    if k == FIELD_ROLL || k == FIELD_PITCH || k == FIELD_DES_ROLL || k == FIELD_DES_PITCH {
        println!("\n=== Field Lookup Debug ===");
        println!("Could not find field '{}' in payload.", k);
        println!("Alternative names tried: {:?}", 
            match k {
                FIELD_ROLL => ALT_ROLL,
                FIELD_PITCH => ALT_PITCH,
                FIELD_DES_ROLL => ALT_DES_ROLL,
                FIELD_DES_PITCH => ALT_DES_PITCH,
                _ => &[]
            }
        );
        
        if let Some(obj) = obj.as_object() {
            // Print all keys in the root object
            println!("\nRoot object has {} keys:", obj.len());
            for (i, key) in obj.keys().enumerate() {
                println!("  {}. {}: {}", i + 1, key, obj[key].type_str());
            }
            
            // If there's a 'values' object, print its contents
            if let Some(values) = obj.get("values").and_then(|v| v.as_object()) {
                println!("\n'values' object has {} keys:", values.len());
                for (i, key) in values.keys().enumerate() {
                    println!("  {}. {}: {}", i + 1, key, values[key].type_str());
                }
            } else {
                println!("\nNo 'values' object found in payload.");
            }
            
            println!("\nSearching for potential angle data in nested objects...");
            for (key, value) in obj {
                if let Some(nested_obj) = value.as_object() {
                    let angle_keys: Vec<_> = nested_obj.keys()
                        .filter(|k| k.to_lowercase().contains("roll") || 
                                 k.to_lowercase().contains("pitch") ||
                                 k.to_lowercase().contains("ang"))
                        .collect();
                    if !angle_keys.is_empty() {
                        println!("  Found potential angle data in '{}': {:?}", key, angle_keys);
                    }
                }
            }
        } else {
            println!("Payload is not an object. Type: {}", obj.type_str());
        }
        println!("=== End Field Lookup Debug ===\n");
    }
    
    None
}

use crate::ws_server::http_server::{AppState, ApiError};

// --- Canonical field names and alternative variants are defined at the top of the file

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

/// Helper function to extract all available numeric fields from a point for debugging
fn debug_point_fields(point: &Value) {
    println!("\n=== Point fields ===");
    
    if let Some(obj) = point.as_object() {
        println!("Root fields ({}): {:?}", obj.len(), obj.keys().collect::<Vec<_>>());
        
        println!("\nNumeric root fields:");
        for (k, v) in obj {
            if v.is_number() {
                println!("  {}: {}", k, v);
            } else if v.is_object() {
                println!("  {}: {{...}} (object)", k);
            } else if v.is_array() {
                println!("  {}: [...] (array, length: {})", k, v.as_array().map(|a| a.len()).unwrap_or(0));
            } else {
                println!("  {}: {:?} ({})", k, v, v.type_str());
            }
        }
    } else {
        println!("Payload is not an object");
    }
    
    // Print all fields in the values object if it exists
    if let Some(values) = point.get("values").and_then(|v| v.as_object()) {
        println!("\nValues object fields ({}): {:?}", values.len(), values.keys().collect::<Vec<_>>());
        
        // Print all numeric fields in values object
        println!("\nNumeric values fields:");
        for (k, v) in values {
            if v.is_number() {
                println!("  values.{}: {}", k, v);
            }
        }
    }
}

#[axum::debug_handler]
pub async fn get_flight_metrics(
    State(state): State<Arc<AppState>>,
    Path(fid): Path<String>,
) -> Result<Json<FlightMetricsResponse>, ApiError> {
    println!("\n=== Computing metrics for flight {} ===", fid);
    
    let ctx = state.ws_ctx.lock().await;
    
    // Fetch all points for this flight
    let points = ctx.questdb
        .fetch_flight_points(&fid, None, None, 1_000_000)
        .await
        .map_err(|e| {
            eprintln!("DB error: {}", e);
            ApiError::Internal(format!("DB error: {}", e))
        })?;

        if points.is_empty() || points.len() < 2 {
            // Responde 200 con métricas vacías; el front no se queda “Cargando…”
            let empty = AngleMetrics {
                rmse_roll: None, rmse_pitch: None,
                itae_roll: None, itae_pitch: None,
                mae_roll: None, mae_pitch: None,
                n_segments_used: 0,
                duration_sec: 0.0,
            };
            let now = chrono::Utc::now();
            let response = FlightMetricsResponse {
                flight_id: fid.clone(),
                start_ts: now.to_rfc3339(),
                end_ts: now.to_rfc3339(),
                duration_sec: 0.0,
                metrics: empty,
                plot_fields: EXTRA_PLOT_FIELDS.iter().map(|s| s.to_string()).collect(),
            };
            return Ok(Json(response));
        }
        
    
    println!("Processing {} samples", points.len());
    
    // Debug: Print fields from first point
    if let Some(first_point) = points.first() {
        debug_point_fields(&first_point.payload);
    }
    
    let t0 = points[0].ts;
    let mut samples = Vec::with_capacity(points.len());
    let mut has_roll_data = false;
    let mut has_pitch_data = false;
    
    // Single pass through points to create samples
    for (i, p) in points.iter().enumerate() {
        let t_rel = (p.ts - t0).num_milliseconds() as f64 / 1000.0;
        let obj = &p.payload;
        
        // Extract all values with fallbacks
        let roll = get_any(obj, FIELD_ROLL, ALT_ROLL);
        let des_roll = get_any(obj, FIELD_DES_ROLL, ALT_DES_ROLL);
        let pitch = get_any(obj, FIELD_PITCH, ALT_PITCH);
        let des_pitch = get_any(obj, FIELD_DES_PITCH, ALT_DES_PITCH);
        
        // Track if we have any valid data
        if roll.is_some() && des_roll.is_some() { has_roll_data = true; }
        if pitch.is_some() && des_pitch.is_some() { has_pitch_data = true; }
        
        samples.push(AngleSample { t_rel, roll, des_roll, pitch, des_pitch });
        
        // Debug first few points
        if i < 3 {
            println!("Sample {}: t={:.3}s roll={:?}° des_roll={:?}° pitch={:?}° des_pitch={:?}°",
                    i, t_rel, 
                    roll.map(|v| v.to_degrees()),
                    des_roll.map(|v| v.to_degrees()),
                    pitch.map(|v| v.to_degrees()),
                    des_pitch.map(|v| v.to_degrees()));
        }
    }
    
    // Calculate valid data duration for each axis
    let mut roll_dt = 0.0;
    let mut pitch_dt = 0.0;
    
    for w in samples.windows(2) {
        let a = &w[0];
        let b = &w[1];
        let dt = (b.t_rel - a.t_rel).max(0.0);
        if dt <= 0.0 { continue; }
        
        if a.roll.is_some() && a.des_roll.is_some() { roll_dt += dt; }
        if a.pitch.is_some() && a.des_pitch.is_some() { pitch_dt += dt; }
    }
    
    let total_duration = samples.last().unwrap().t_rel - samples.first().unwrap().t_rel;
    println!("\nData validation:");
    println!("  Total duration: {:.3}s", total_duration);
    println!("  Roll data: {:.1}% ({:.3}s) {}", 
             (roll_dt / total_duration * 100.0), roll_dt,
             if has_roll_data { "✓" } else { "✗ No valid roll+desired_roll pairs" });
    println!("  Pitch data: {:.1}% ({:.3}s) {}", 
             (pitch_dt / total_duration * 100.0), pitch_dt,
             if has_pitch_data { "✓" } else { "✗ No valid pitch+desired_pitch pairs" });
             
             if !has_roll_data && !has_pitch_data {
                // Responder con métricas vacías pero 200
                let response = FlightMetricsResponse {
                    flight_id: fid.clone(),
                    start_ts: points.first().unwrap().ts.to_rfc3339(),
                    end_ts: points.last().unwrap().ts.to_rfc3339(),
                    duration_sec: (points.last().unwrap().ts - points.first().unwrap().ts).num_milliseconds() as f64 / 1000.0,
                    metrics: AngleMetrics {
                        rmse_roll: None, rmse_pitch: None,
                        itae_roll: None, itae_pitch: None,
                        mae_roll: None, mae_pitch: None,
                        n_segments_used: 0,
                        duration_sec: 0.0,
                    },
                    plot_fields: EXTRA_PLOT_FIELDS.iter().map(|s| s.to_string()).collect(),
                };
                return Ok(Json(response));
            }
            
    
    // Calculate metrics
    let metrics = compute_angle_metrics(&samples);
    
    // Clone metrics for debug prints
    let metrics_clone = metrics.clone();
    
    // Prepare response
    let response = FlightMetricsResponse {
        flight_id: fid.clone(),
        start_ts: points.first().unwrap().ts.to_rfc3339(),
        end_ts: points.last().unwrap().ts.to_rfc3339(),
        duration_sec: total_duration,
        metrics,
        plot_fields: EXTRA_PLOT_FIELDS.iter().map(|s| s.to_string()).collect(),
    };
    
    // Use the cloned metrics for debug prints
    let metrics = metrics_clone;
    
    if has_roll_data {
    }
    if has_pitch_data {
    }
    
    Ok(Json(response))
}

pub fn compute_angle_metrics(samples: &[AngleSample]) -> AngleMetrics {
    println!("\n=== Computing metrics for {} samples ===", samples.len());
    
    if samples.len() < 2 {
        println!("Not enough samples ({} < 2)", samples.len());
        return AngleMetrics { rmse_roll: None, rmse_pitch: None, itae_roll: None, itae_pitch: None,
                              mae_roll: None, mae_pitch: None, n_segments_used: 0, duration_sec: 0.0 };
    }

    let duration_sec = samples.last().unwrap().t_rel - samples.first().unwrap().t_rel;
    println!("Duration: {:.3} seconds", duration_sec);
    
    if duration_sec <= 0.0 {
        println!("Invalid duration: {}", duration_sec);
        return AngleMetrics { rmse_roll: None, rmse_pitch: None, itae_roll: None, itae_pitch: None,
                              mae_roll: None, mae_pitch: None, n_segments_used: 0, duration_sec: 0.0 };
    }

    let mut sum_abs_roll_dt  = 0.0;
    let mut sum_sq_roll_dt   = 0.0;
    let mut sum_itae_roll    = 0.0;
    let mut roll_dt_total    = 0.0;
    let mut roll_used        = 0usize;

    let mut sum_abs_pitch_dt = 0.0;
    let mut sum_sq_pitch_dt  = 0.0;
    let mut sum_itae_pitch   = 0.0;
    let mut pitch_dt_total   = 0.0;
    let mut pitch_used       = 0usize;

    for w in samples.windows(2) {
        let a = &w[0];
        let b = &w[1];
        let dt = (b.t_rel - a.t_rel).max(0.0);
        if dt <= 0.0 { continue; }

        // Roll
        if let (Some(r_a), Some(dr_a)) = (a.roll, a.des_roll) {
            let e = r_a - dr_a;
            sum_abs_roll_dt += e.abs() * dt;
            sum_sq_roll_dt  += e * e * dt;
            sum_itae_roll   += a.t_rel * e.abs() * dt;  // ITAE con t del extremo izquierdo
            roll_dt_total   += dt;
            roll_used       += 1;
            
            if roll_used <= 3 {  // Print first few calculations for debugging
                println!("Roll[{}]: t={:.3}, roll={:.3}, des_roll={:.3}, e={:.3}, dt={:.3}", 
                        roll_used, a.t_rel, r_a, dr_a, e, dt);
            }
        }

        // Pitch
        if let (Some(p_a), Some(dp_a)) = (a.pitch, a.des_pitch) {
            let e = p_a - dp_a;
            sum_abs_pitch_dt += e.abs() * dt;
            sum_sq_pitch_dt  += e * e * dt;
            sum_itae_pitch   += a.t_rel * e.abs() * dt;
            pitch_dt_total   += dt;
            pitch_used       += 1;
            
            if pitch_used <= 3 {  // Print first few calculations for debugging
                println!("Pitch[{}]: t={:.3}, pitch={:.3}, des_pitch={:.3}, e={:.3}, dt={:.3}", 
                        pitch_used, a.t_rel, p_a, dp_a, e, dt);
            }
        }
    }

    // Calculate metrics with debug output
    let mae_roll = if roll_dt_total > 0.0 {
        let val = sum_abs_roll_dt / roll_dt_total;
        Some(val)
    } else {
        None
    };
    
    let rmse_roll = if roll_dt_total > 0.0 {
        let val = (sum_sq_roll_dt / roll_dt_total).sqrt();
        Some(val)
    } else {
        None
    };
    
    let itae_roll = if roll_used > 0 {
        Some(sum_itae_roll)
    } else {
        None
    };
    
    let mae_pitch = if pitch_dt_total > 0.0 {
        let val = sum_abs_pitch_dt / pitch_dt_total;
       //println!("Pitch MAE: {}", val);
        Some(val)
    } else {
        //println!("Pitch MAE: No valid data (total time: {})", pitch_dt_total);
        None
    };
    
    let rmse_pitch = if pitch_dt_total > 0.0 {
        let val = (sum_sq_pitch_dt / pitch_dt_total).sqrt();
       //println!("Pitch RMSE: {}", val);
        Some(val)
    } else {
        //println!("Pitch RMSE: No valid data (total time: {})", pitch_dt_total);
        None
    };
    
    let itae_pitch = if pitch_used > 0 {
        //println!("Pitch ITAE: {}", sum_itae_pitch);
        Some(sum_itae_pitch)
    } else {
        //println!("Pitch ITAE: No valid data (samples: {})", pitch_used);
        None
    };

    AngleMetrics {
        rmse_roll, rmse_pitch, itae_roll, itae_pitch,
        mae_roll, mae_pitch,
        n_segments_used: roll_used + pitch_used,
        duration_sec,
    }
}
