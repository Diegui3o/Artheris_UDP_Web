use serde::Serialize;

/// Predicción de tendencia del error
#[derive(Debug, Clone, Serialize)]
pub struct TrendPrediction {
    pub error_type: String,              // "roll" o "pitch"
    pub last_error: f64,                 // Último error registrado
    pub predicted_error: f64,            // Error predicho
    pub seconds_ahead: f64,              // Segundos hacia adelante
    pub slope: f64,                      // Pendiente de la regresión
    pub trend: TrendDirection,            // Dirección de la tendencia
    pub confidence: f64,                 // Confianza (0-1)
    pub recommendation: String,          // Recomendación basada en la tendencia
}

/// Dirección de la tendencia
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum TrendDirection {
    #[serde(rename = "aumentando")]
    Aumentando,
    #[serde(rename = "disminuyendo")]
    Disminuyendo,
    #[serde(rename = "estable")]
    Estable,
}

/// Reporte completo de predicciones
#[derive(Debug, Clone, Serialize)]
pub struct TrendReport {
    pub flight_id: String,
    pub roll_trend: Option<TrendPrediction>,
    pub pitch_trend: Option<TrendPrediction>,
    pub overall_assessment: String,
    pub warnings: Vec<String>,
}

/// Calcula la tendencia de una señal usando regresión lineal
pub fn compute_trend(
    values: &[(f64, f64)],  // (timestamp, value)
    seconds_ahead: f64,
    error_type: &str,
    threshold: f64,          // Umbral para considerar "aumentando"
) -> Option<TrendPrediction> {
    if values.len() < 10 {
        return None;
    }
    
    let n = values.len();
    let sample_rate = if n > 1 {
        let total_time = values.last().unwrap().0 - values.first().unwrap().0;
        if total_time > 0.0 {
            n as f64 / total_time
        } else {
            25.0  // fallback
        }
    } else {
        25.0
    };
    
    let steps_ahead = (seconds_ahead * sample_rate) as usize;
    
    // Preparar datos para regresión (usando índices en lugar de timestamps para simplificar)
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xy = 0.0;
    let mut sum_xx = 0.0;
    
    for (i, (_, val)) in values.iter().enumerate() {
        let x = i as f64;
        let y = *val;
        sum_x += x;
        sum_y += y;
        sum_xy += x * y;
        sum_xx += x * x;
    }
    
    // Calcular pendiente (slope) e intercepto
    let denominator = n as f64 * sum_xx - sum_x * sum_x;
    if denominator.abs() < 1e-10 {
        return None;
    }
    
    let slope = (n as f64 * sum_xy - sum_x * sum_y) / denominator;
    let intercept = (sum_y - slope * sum_x) / n as f64;
    
    let last_index = (n - 1) as f64;
    let last_error = values.last().unwrap().1;
    let predicted_error = intercept + slope * (last_index + steps_ahead as f64);
    
    // Determinar dirección de la tendencia
    let trend = if slope > threshold {
        TrendDirection::Aumentando
    } else if slope < -threshold {
        TrendDirection::Disminuyendo
    } else {
        TrendDirection::Estable
    };
    
    // Calcular confianza (basado en R² aproximado)
    let mut ss_res = 0.0;
    let mut ss_tot = 0.0;
    let mean_y = sum_y / n as f64;
    
    for (i, (_, val)) in values.iter().enumerate() {
        let x = i as f64;
        let y_pred = intercept + slope * x;
        ss_res += (y_pred - val).powi(2);
        ss_tot += (val - mean_y).powi(2);
    }
    
    let r_squared = if ss_tot > 0.0 { 1.0 - (ss_res / ss_tot) } else { 0.0 };
    let confidence = r_squared.clamp(0.0, 0.95);
    
    // Generar recomendación
    let recommendation = match trend {
        TrendDirection::Aumentando => {
            if predicted_error.abs() > 2.0 {
                format!("⚠️ El error en {} está aumentando (pendiente {:.3}°/s). Se espera que alcance {:.2}° en {:.1}s. Revisar controlador.", 
                        error_type, slope, predicted_error.abs(), seconds_ahead)
            } else {
                format!("📈 El error en {} está aumentando lentamente (pendiente {:.3}°/s). Monitorear evolución.", 
                        error_type, slope)
            }
        }
        TrendDirection::Disminuyendo => {
            format!("📉 El error en {} está disminuyendo (pendiente {:.3}°/s). Buena tendencia.", 
                    error_type, slope)
        }
        TrendDirection::Estable => {
            format!("➡️ El error en {} se mantiene estable (pendiente {:.3}°/s). Comportamiento esperado.", 
                    error_type, slope)
        }
    };
    
    Some(TrendPrediction {
        error_type: error_type.to_string(),
        last_error,
        predicted_error,
        seconds_ahead,
        slope,
        trend,
        confidence,
        recommendation,
    })
}

/// Analiza la tendencia de un vuelo completo
pub fn analyze_trends(
    flight_id: &str,
    roll_errors: &[(f64, f64)],   // (timestamp, error)
    pitch_errors: &[(f64, f64)],
    seconds_ahead: f64,
) -> TrendReport {
    let mut warnings = Vec::new();
    
    let roll_trend = compute_trend(roll_errors, seconds_ahead, "Roll", 0.05);
    let pitch_trend = compute_trend(pitch_errors, seconds_ahead, "Pitch", 0.05);
    
    // Evaluación general
    let overall_assessment = match (&roll_trend, &pitch_trend) {
        (Some(r), Some(p)) => {
            if r.trend == TrendDirection::Aumentando && p.trend == TrendDirection::Aumentando {
                warnings.push("⚠️ Ambos ejes (Roll y Pitch) muestran tendencia al aumento. Revisar estabilidad del sistema.".to_string());
                "Tendencia crítica: ambos errores están aumentando".to_string()
            } else if r.trend == TrendDirection::Aumentando {
                warnings.push("⚠️ El error en Roll está aumentando. Posible problema de control.".to_string());
                "Tendencia moderada: Roll aumentando, Pitch estable".to_string()
            } else if p.trend == TrendDirection::Aumentando {
                warnings.push("⚠️ El error en Pitch está aumentando. Posible problema de control.".to_string());
                "Tendencia moderada: Pitch aumentando, Roll estable".to_string()
            } else if r.trend == TrendDirection::Disminuyendo && p.trend == TrendDirection::Disminuyendo {
                "✅ Excelente: ambos errores están disminuyendo".to_string()
            } else {
                "✅ Tendencia estable en ambos ejes".to_string()
            }
        }
        (Some(r), None) => {
            if r.trend == TrendDirection::Aumentando {
                warnings.push("⚠️ El error en Roll está aumentando.".to_string());
                "Tendencia crítica: Roll aumentando".to_string()
            } else {
                "Tendencia estable en Roll".to_string()
            }
        }
        (None, Some(p)) => {
            if p.trend == TrendDirection::Aumentando {
                warnings.push("⚠️ El error en Pitch está aumentando.".to_string());
                "Tendencia crítica: Pitch aumentando".to_string()
            } else {
                "Tendencia estable en Pitch".to_string()
            }
        }
        _ => "No hay suficientes datos para análisis de tendencia".to_string(),
    };
    
    TrendReport {
        flight_id: flight_id.to_string(),
        roll_trend,
        pitch_trend,
        overall_assessment,
        warnings,
    }
}