use crate::analysis::anomaly::AnomalyReport;
use crate::config::metrics::FullFlightMetrics;
use crate::config::spectrum_types::FlightSpectrum;
use serde::Serialize;

/// Prioridad de la recomendación
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum Priority {
    #[serde(rename = "crítica")]
    Critica,
    #[serde(rename = "alta")]
    Alta,
    #[serde(rename = "media")]
    Media,
    #[serde(rename = "baja")]
    Baja,
    #[serde(rename = "informativa")]
    Informativa,
}

/// Área de mejora
#[derive(Debug, Clone, Serialize)]
pub enum Area {
    #[serde(rename = "filtro_kalman")]
    FiltroKalman,
    #[serde(rename = "vibraciones")]
    Vibraciones,
    #[serde(rename = "ruido_sensor")]
    RuidoSensor,
    #[serde(rename = "controlador")]
    Controlador,
    #[serde(rename = "telemetria")]
    Telemetria,
    #[serde(rename = "calibracion")]
    Calibracion,
    #[serde(rename = "estabilidad")]
    Estabilidad,
}

/// Una recomendación de mejora
#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub id: u32,
    pub priority: Priority,
    pub area: Area,
    pub title: String,
    pub description: String,
    pub suggestion: String,
    pub expected_improvement: Option<String>,
    pub action_items: Vec<String>,
    pub metrics_affected: Vec<String>,
}

/// Reporte completo de recomendaciones
#[derive(Debug, Clone, Serialize)]
pub struct RecommendationsReport {
    pub flight_id: String,
    pub flight_type: String,
    pub overall_score: u8,
    pub recommendations: Vec<Recommendation>,
    pub summary: String,
    pub quick_wins: Vec<Recommendation>,
}

#[allow(unused_assignments)]
pub fn generate_recommendations(
    flight_id: &str,
    flight_type: &str,
    metrics: &FullFlightMetrics,
    spectrum: &FlightSpectrum,
    anomalies: &AnomalyReport,
) -> RecommendationsReport {
    let mut recommendations = Vec::new();
    let mut rec_id = 1;
    
    // ==================== 1. ANÁLISIS DEL FILTRO KALMAN ====================
    let improvement_roll = metrics.comparison_roll.improvement_percent.unwrap_or(0.0);
    let improvement_pitch = metrics.comparison_pitch.improvement_percent.unwrap_or(0.0);
    let avg_improvement = (improvement_roll + improvement_pitch) / 2.0;
    
    if avg_improvement < 10.0 {
        recommendations.push(Recommendation {
            id: rec_id,
            priority: Priority::Alta,
            area: Area::FiltroKalman,
            title: "Sintonización del Filtro Kalman".to_string(),
            description: format!(
                "La mejora del filtro Kalman es baja ({:.1}%). El filtro no está reduciendo significativamente el ruido.",
                avg_improvement
            ),
            suggestion: "Revisar los parámetros Q (ruido de proceso) y R (ruido de medición) del filtro Kalman.".to_string(),
            expected_improvement: Some(format!("Mejora esperada: +{}%", (50.0 - avg_improvement).max(0.0) as i32)),
            action_items: vec![
                "Reducir R_measure (ruido de medición) para dar más peso a la medición del ángulo".to_string(),
                "Aumentar Q_angle para permitir más variación en el ángulo estimado".to_string(),
                "Verificar que la frecuencia de muestreo (dt) sea correcta en el filtro".to_string(),
            ],
            metrics_affected: vec!["rmse_roll".to_string(), "rmse_pitch".to_string()],
        });
        rec_id += 1;
    } else if avg_improvement > 30.0 {
        recommendations.push(Recommendation {
            id: rec_id,
            priority: Priority::Informativa,
            area: Area::FiltroKalman,
            title: "Filtro Kalman Bien Sintonizado".to_string(),
            description: format!(
                "El filtro Kalman está funcionando correctamente con una mejora del {:.1}%.",
                avg_improvement
            ),
            suggestion: "Mantener la configuración actual del filtro Kalman.".to_string(),
            expected_improvement: None,
            action_items: vec![],
            metrics_affected: vec![],
        });
        rec_id += 1;
    }
    
    // ==================== 2. ANÁLISIS DE VIBRACIONES ====================
    if let Some(peak) = spectrum.error_spectrum.dominant_peaks.first() {
        if peak.magnitude > 0.5 {
            let priority = if peak.magnitude > 2.0 {
                Priority::Critica
            } else if peak.magnitude > 1.0 {
                Priority::Alta
            } else {
                Priority::Media
            };
            
            recommendations.push(Recommendation {
                id: rec_id,
                priority,
                area: Area::Vibraciones,
                title: format!("Vibración Detectada a {:.1} Hz", peak.frequency_hz),
                description: format!(
                    "Se detectó una vibración dominante de {:.1} Hz con magnitud {:.2}. Esta frecuencia puede estar afectando la estimación de actitud.",
                    peak.frequency_hz, peak.magnitude
                ),
                suggestion: format!(
                    "Implementar un filtro notch en {:.1} Hz para atenuar la vibración.",
                    peak.frequency_hz
                ),
                expected_improvement: Some(format!("Reducción de error RMS estimada: {:.1}%", (peak.magnitude * 20.0).min(50.0))),
                action_items: vec![
                    format!("Agregar filtro notch en {:.1} Hz en el loop de control", peak.frequency_hz),
                    "Verificar el balanceo de las hélices".to_string(),
                    "Revisar la fijación mecánica del sensor IMU".to_string(),
                ],
                metrics_affected: vec!["rmse_roll".to_string(), "rmse_pitch".to_string()],
            });
            rec_id += 1;
        }
    }
    
    // ==================== 3. ANÁLISIS DE RUIDO DEL SENSOR ====================
    let raw_rmse_roll = metrics.comparison_roll.raw_rms.unwrap_or(0.0);
    let kalman_rmse_roll = metrics.comparison_roll.kalman_rms.unwrap_or(0.0);
    let raw_vs_kalman_ratio = if raw_rmse_roll > 0.0 {
        kalman_rmse_roll / raw_rmse_roll
    } else {
        1.0
    };
    
    if raw_vs_kalman_ratio > 0.8 {
        recommendations.push(Recommendation {
            id: rec_id,
            priority: Priority::Media,
            area: Area::RuidoSensor,
            title: "Ruido Elevado en el Sensor IMU".to_string(),
            description: format!(
                "El filtro Kalman reduce el ruido solo en un {:.1}%. El sensor IMU puede tener ruido excesivo.",
                (1.0 - raw_vs_kalman_ratio) * 100.0
            ),
            suggestion: "Verificar la calidad del sensor IMU y el aislamiento de vibraciones.".to_string(),
            expected_improvement: Some("Reducción de ruido base del 50-70%".to_string()),
            action_items: vec![
                "Agregar un filtro pasa-bajos antes del Kalman".to_string(),
                "Verificar la alimentación del sensor (ruido en la fuente)".to_string(),
                "Aislar mecánicamente el sensor IMU de vibraciones".to_string(),
            ],
            metrics_affected: vec!["raw_rms".to_string(), "kalman_rms".to_string()],
        });
        rec_id += 1;
    }
    
    // ==================== 4. ANÁLISIS DE ANOMALÍAS ====================
    if anomalies.total_anomalies > 0 {
        let priority = if anomalies.summary.max_severity > 80.0 {
            Priority::Critica
        } else if anomalies.total_anomalies > 50 {
            Priority::Alta
        } else {
            Priority::Media
        };
        
        recommendations.push(Recommendation {
            id: rec_id,
            priority,
            area: Area::Telemetria,
            title: format!("{} Anomalías Detectadas", anomalies.total_anomalies),
            description: format!(
                "Se detectaron {} anomalías durante el vuelo, con severidad máxima del {:.0}%.",
                anomalies.total_anomalies, anomalies.summary.max_severity
            ),
            suggestion: "Revisar la calidad de los datos y la integridad de la transmisión.".to_string(),
            expected_improvement: Some("Calidad de datos mejorada".to_string()),
            action_items: vec![
                "Verificar la tasa de muestreo y pérdida de paquetes".to_string(),
                "Implementar filtros anti-aliasing en el ADC".to_string(),
                "Agregar validación de datos (rango válido) en el firmware".to_string(),
            ],
            metrics_affected: vec!["sample_count".to_string(), "error_rmse".to_string()],
        });
        rec_id += 1;
    }
    
    // ==================== 5. ANÁLISIS DE ESTABILIDAD ====================
    let std_dev_roll = metrics.error_metrics.std_dev_roll.unwrap_or(0.0);
    let std_dev_pitch = metrics.error_metrics.std_dev_pitch.unwrap_or(0.0);
    
    if std_dev_roll > 1.0 || std_dev_pitch > 1.0 {
        recommendations.push(Recommendation {
            id: rec_id,
            priority: Priority::Media,
            area: Area::Estabilidad,
            title: "Alta Variabilidad en el Error de Seguimiento".to_string(),
            description: format!(
                "La desviación estándar del error es de {:.2}° en roll y {:.2}° en pitch, indicando comportamiento inestable.",
                std_dev_roll, std_dev_pitch
            ),
            suggestion: "Revisar la ganancia del controlador PID para mejorar la estabilidad.".to_string(),
            expected_improvement: Some("Reducción de varianza del 30-50%".to_string()),
            action_items: vec![
                "Aumentar la ganancia proporcional (Kp) para respuesta más rápida".to_string(),
                "Ajustar la ganancia derivativa (Kd) para reducir oscilaciones".to_string(),
                "Verificar la sintonización del controlador en vuelo estacionario".to_string(),
            ],
            metrics_affected: vec!["std_dev_roll".to_string(), "std_dev_pitch".to_string()],
        });
        rec_id += 1;
    }
    
    // ==================== 6. ANÁLISIS DE FRECUENCIA DE MUESTREO ====================
    if spectrum.sample_rate_hz < 20.0 {
        recommendations.push(Recommendation {
            id: rec_id,
            priority: Priority::Alta,
            area: Area::Telemetria,
            title: "Frecuencia de Muestreo Baja".to_string(),
            description: format!(
                "La frecuencia de muestreo detectada es de {:.1} Hz. Esto limita el análisis de vibraciones a frecuencias < {:.1} Hz.",
                spectrum.sample_rate_hz, spectrum.sample_rate_hz / 2.0
            ),
            suggestion: "Aumentar la frecuencia de telemetría para capturar más dinámica.".to_string(),
            expected_improvement: Some("Análisis espectral completo hasta 50 Hz".to_string()),
            action_items: vec![
                "Reducir el período de envío de telemetría en el ESP32".to_string(),
                "Optimizar el formato de datos (binario en lugar de JSON)".to_string(),
                "Usar UDP en lugar de WiFi si es posible".to_string(),
            ],
            metrics_affected: vec!["spectrum".to_string()],
        });
        rec_id += 1;
    }
    
    // ==================== 7. RESULTADOS POSITIVOS ====================
    let mut positive_insights = Vec::new();
    
    if improvement_pitch > 40.0 {
        positive_insights.push(format!("✅ Excelente mejora del Kalman en pitch ({:.1}%)", improvement_pitch));
    }
    
    if metrics.error_metrics.rmse_roll.unwrap_or(0.0) < 0.5 {
        positive_insights.push("✅ Error de seguimiento excelente (<0.5°)".to_string());
    }
    
    if anomalies.total_anomalies == 0 {
        positive_insights.push("✅ Sin anomalías detectadas. Datos de alta calidad.".to_string());
    }
    
    // Resumen general
    let summary = if !positive_insights.is_empty() {
        format!("Vuelo de tipo {} con {}. Mejoras sugeridas: {}.",
            flight_type,
            positive_insights.join(" "),
            recommendations.iter().map(|r| r.title.clone()).collect::<Vec<_>>().join(", ")
        )
    } else {
        format!("Vuelo de tipo {} con {} recomendaciones para mejorar.",
            flight_type,
            recommendations.len()
        )
    };
    
    // Quick wins (recomendaciones de prioridad alta y crítica)
    let quick_wins: Vec<Recommendation> = recommendations.iter()
        .filter(|r| r.priority == Priority::Critica || r.priority == Priority::Alta)
        .cloned()
        .collect();
    
    // Calcular score general (0-100)
    let overall_score = calculate_overall_score(metrics, anomalies, &recommendations);
    
    RecommendationsReport {
        flight_id: flight_id.to_string(),
        flight_type: flight_type.to_string(),
        overall_score,
        recommendations,
        summary,
        quick_wins,
    }
}

/// Calcula un score general de calidad (0-100)
fn calculate_overall_score(
    metrics: &FullFlightMetrics,
    anomalies: &AnomalyReport,
    recommendations: &[Recommendation],
) -> u8 {
    let mut score = 100.0;
    
    // Penalizar por errores altos
    let rmse_roll = metrics.error_metrics.rmse_roll.unwrap_or(0.0);
    score -= (rmse_roll * 5.0).min(30.0);
    
    // Penalizar por anomalías
    score -= (anomalies.total_anomalies as f64 * 0.5).min(20.0);
    
    // Bonificar por mejora del Kalman
    let improvement = metrics.comparison_roll.improvement_percent.unwrap_or(0.0);
    score += (improvement / 5.0).min(20.0);
    
    // Penalizar por recomendaciones críticas
    let critical_count = recommendations.iter()
        .filter(|r| r.priority == Priority::Critica)
        .count() as f64;
    score -= critical_count * 10.0;
    
    score.max(0.0).min(100.0) as u8
}