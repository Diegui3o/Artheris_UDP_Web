use crate::analysis::anomaly::AnomalyReport;
use crate::config::metrics::FullFlightMetrics;
use crate::config::spectrum_types::FlightSpectrum;
use serde::Serialize;

/// Categoría de calidad del vuelo
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum QualityCategory {
    #[serde(rename = "excelente")]
    Excelente,      // 90-100
    #[serde(rename = "bueno")]
    Bueno,          // 70-89
    #[serde(rename = "aceptable")]
    Aceptable,      // 50-69
    #[serde(rename = "deficiente")]
    Deficiente,     // 30-49
    #[serde(rename = "muy_deficiente")]
    MuyDeficiente,  // 0-29
}

/// Resultado completo del score de calidad
#[derive(Debug, Clone, Serialize)]
pub struct QualityScore {
    pub total_score: u8,
    pub category: QualityCategory,
    pub breakdown: ScoreBreakdown,
    pub recommendations_summary: Vec<String>,
}

/// Desglose del score por categoría
#[derive(Debug, Clone, Serialize)]
pub struct ScoreBreakdown {
    pub error_score: u8,        // 0-30 puntos
    pub noise_score: u8,        // 0-20 puntos
    pub kalman_score: u8,       // 0-20 puntos
    pub stability_score: u8,    // 0-15 puntos
    pub anomaly_score: u8,      // 0-15 puntos
    pub total: u8,
}

/// Calcula el score de calidad de un vuelo
pub fn compute_quality_score(
    metrics: &FullFlightMetrics,
    _spectrum: &FlightSpectrum,
    anomalies: &AnomalyReport,
) -> QualityScore {
    let mut breakdown = ScoreBreakdown {
        error_score: 0,
        noise_score: 0,
        kalman_score: 0,
        stability_score: 0,
        anomaly_score: 0,
        total: 0,
    };
    let mut recommendations = Vec::new();
    
    // ==================== 1. SCORE DE ERROR (máx 30 puntos) ====================
    let rmse_roll = metrics.error_metrics.rmse_roll.unwrap_or(1.0);
    let rmse_pitch = metrics.error_metrics.rmse_pitch.unwrap_or(1.0);
    let avg_rmse = (rmse_roll + rmse_pitch) / 2.0;
    
    let error_score = if avg_rmse < 0.5 {
        recommendations.push("✅ Error de seguimiento excelente (<0.5°)".to_string());
        30
    } else if avg_rmse < 1.0 {
        recommendations.push("✅ Error de seguimiento bueno (<1.0°)".to_string());
        25
    } else if avg_rmse < 2.0 {
        recommendations.push("⚠️ Error de seguimiento aceptable (1-2°)".to_string());
        20
    } else if avg_rmse < 5.0 {
        recommendations.push("⚠️ Error de seguimiento elevado (2-5°)".to_string());
        15
    } else if avg_rmse < 10.0 {
        recommendations.push("❌ Error de seguimiento muy elevado (5-10°)".to_string());
        8
    } else {
        recommendations.push("❌ Error de seguimiento crítico (>10°)".to_string());
        0
    };
    
    breakdown.error_score = error_score;
    
    // ==================== 2. SCORE DE RUIDO (máx 20 puntos) ====================
    let raw_rmse = metrics.comparison_roll.raw_rms.unwrap_or(1.0);
    let kalman_rmse = metrics.comparison_roll.kalman_rms.unwrap_or(1.0);
    let noise_reduction = if raw_rmse > 0.0 {
        ((raw_rmse - kalman_rmse) / raw_rmse) * 100.0
    } else {
        0.0
    };
    
    let noise_score = if noise_reduction > 70.0 {
        recommendations.push("✅ Excelente reducción de ruido (>70%)".to_string());
        20
    } else if noise_reduction > 50.0 {
        recommendations.push("✅ Buena reducción de ruido (50-70%)".to_string());
        16
    } else if noise_reduction > 30.0 {
        recommendations.push("⚠️ Reducción de ruido aceptable (30-50%)".to_string());
        12
    } else if noise_reduction > 10.0 {
        recommendations.push("⚠️ Baja reducción de ruido (10-30%)".to_string());
        8
    } else {
        recommendations.push("❌ Muy baja reducción de ruido (<10%)".to_string());
        0
    };
    
    breakdown.noise_score = noise_score;
    
    // ==================== 3. SCORE DEL KALMAN (máx 20 puntos) ====================
    let improvement = metrics.comparison_roll.improvement_percent.unwrap_or(0.0);
    let improvement_pitch = metrics.comparison_pitch.improvement_percent.unwrap_or(0.0);
    let avg_improvement = (improvement + improvement_pitch) / 2.0;
    
    let kalman_score = if avg_improvement > 50.0 {
        recommendations.push("✅ Filtro Kalman excelente (>50% mejora)".to_string());
        20
    } else if avg_improvement > 40.0 {
        recommendations.push("✅ Filtro Kalman muy bueno (40-50%)".to_string());
        18
    } else if avg_improvement > 30.0 {
        recommendations.push("✅ Filtro Kalman bueno (30-40%)".to_string());
        15
    } else if avg_improvement > 20.0 {
        recommendations.push("⚠️ Filtro Kalman aceptable (20-30%)".to_string());
        12
    } else if avg_improvement > 10.0 {
        recommendations.push("⚠️ Filtro Kalman bajo (10-20%)".to_string());
        8
    } else {
        recommendations.push("❌ Filtro Kalman no mejora significativamente (<10%)".to_string());
        0
    };
    
    breakdown.kalman_score = kalman_score;
    
    // ==================== 4. SCORE DE ESTABILIDAD (máx 15 puntos) ====================
    let std_dev_roll = metrics.error_metrics.std_dev_roll.unwrap_or(1.0);
    let std_dev_pitch = metrics.error_metrics.std_dev_pitch.unwrap_or(1.0);
    let avg_std_dev = (std_dev_roll + std_dev_pitch) / 2.0;
    
    let stability_score = if avg_std_dev < 0.2 {
        recommendations.push("✅ Estabilidad excelente (σ<0.2°)".to_string());
        15
    } else if avg_std_dev < 0.5 {
        recommendations.push("✅ Estabilidad muy buena (σ<0.5°)".to_string());
        12
    } else if avg_std_dev < 1.0 {
        recommendations.push("✅ Estabilidad buena (σ<1.0°)".to_string());
        9
    } else if avg_std_dev < 2.0 {
        recommendations.push("⚠️ Estabilidad aceptable (σ<2.0°)".to_string());
        6
    } else if avg_std_dev < 5.0 {
        recommendations.push("⚠️ Estabilidad baja (σ<5.0°)".to_string());
        3
    } else {
        recommendations.push("❌ Estabilidad muy baja (σ>5.0°)".to_string());
        0
    };
    
    breakdown.stability_score = stability_score;
    
    // ==================== 5. SCORE DE ANOMALÍAS (máx 15 puntos) ====================
    let anomaly_count = anomalies.total_anomalies;
    let max_severity = anomalies.summary.max_severity;
    
    let anomaly_score = if anomaly_count == 0 {
        recommendations.push("✅ Sin anomalías detectadas".to_string());
        15
    } else if anomaly_count < 10 && max_severity < 30.0 {
        recommendations.push("✅ Pocas anomalías de baja severidad".to_string());
        12
    } else if anomaly_count < 50 && max_severity < 50.0 {
        recommendations.push("⚠️ Algunas anomalías de severidad moderada".to_string());
        8
    } else if anomaly_count < 100 {
        recommendations.push("⚠️ Múltiples anomalías detectadas".to_string());
        4
    } else {
        recommendations.push("❌ Muchas anomalías. Revisar calidad de datos.".to_string());
        0
    };
    
    breakdown.anomaly_score = anomaly_score;
    
    // ==================== SCORE TOTAL ====================
    let total_score = error_score + noise_score + kalman_score + stability_score + anomaly_score;
    breakdown.total = total_score;
    
    // Determinar categoría
    let category = match total_score {
        90..=100 => QualityCategory::Excelente,
        70..=89 => QualityCategory::Bueno,
        50..=69 => QualityCategory::Aceptable,
        30..=49 => QualityCategory::Deficiente,
        _ => QualityCategory::MuyDeficiente,
    };
    
    QualityScore {
        total_score,
        category,
        breakdown,
        recommendations_summary: recommendations,
    }
}

/// Versión simplificada para obtener solo el score numérico
pub fn quick_score(metrics: &FullFlightMetrics, spectrum: &FlightSpectrum, anomalies: &AnomalyReport) -> u8 {
    compute_quality_score(metrics, spectrum, anomalies).total_score
}