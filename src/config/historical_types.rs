use serde::Serialize;
use crate::config::metrics::FullFlightMetrics;
use std::collections::HashMap;

/// Estadísticas históricas para un tipo de vuelo
#[derive(Debug, Clone, Serialize)]
pub struct HistoricalStats {
    pub flight_type: String,
    pub sample_count: usize,
    pub metrics: HashMap<String, MetricStats>,
    pub percentiles: PercentileStats,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

/// Estadísticas de una métrica individual
#[derive(Debug, Clone, Serialize)]
pub struct MetricStats {
    pub mean: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
    pub median: f64,
}

/// Percentiles principales
#[derive(Debug, Clone, Serialize)]
pub struct PercentileStats {
    pub p5: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
}

/// Comparación de un vuelo actual con el histórico
#[derive(Debug, Clone, Serialize)]
pub struct HistoricalComparison {
    pub flight_id: String,
    pub flight_type: String,
    
    /// Métricas del vuelo actual
    pub current: FlightComparisonMetrics,
    
    /// Estadísticas históricas para este tipo de vuelo
    pub historical: HistoricalStats,
    
    /// Comparación detallada
    pub comparison: ComparisonDetails,
    
    /// Puntuación de calidad (0-100)
    pub quality_score: f64,
    
    /// Ranking entre vuelos similares
    pub rank: RankInfo,
}

/// Métricas del vuelo actual para comparación
#[derive(Debug, Clone, Serialize)]
pub struct FlightComparisonMetrics {
    pub rmse_roll: Option<f64>,
    pub rmse_pitch: Option<f64>,
    pub improvement_percent: Option<f64>,
    pub variance_roll: Option<f64>,
    pub variance_pitch: Option<f64>,
    pub itae_roll: Option<f64>,
    pub itae_pitch: Option<f64>,
}

/// Detalles de comparación con percentiles
#[derive(Debug, Clone, Serialize)]
pub struct ComparisonDetails {
    pub rmse_roll: MetricComparison,
    pub rmse_pitch: MetricComparison,
    pub improvement_percent: MetricComparison,
    pub variance_roll: MetricComparison,
    pub variance_pitch: MetricComparison,
}

/// Comparación de una métrica específica
#[derive(Debug, Clone, Serialize)]
pub struct MetricComparison {
    pub value: f64,
    pub mean: f64,
    pub std_dev: f64,
    pub z_score: f64,                 // Desviaciones estándar respecto a la media
    pub percentile: f64,              // Percentil en el que se encuentra
    pub better_than_average: bool,
    pub interpretation: String,       // Texto interpretativo
}

/// Información de ranking entre vuelos similares
#[derive(Debug, Clone, Serialize)]
pub struct RankInfo {
    pub position: usize,              // Posición (1 = mejor)
    pub total: usize,                 // Total de vuelos en la categoría
    pub percentile: f64,              // Percentil de rendimiento
    pub label: String,                // "Excelente", "Bueno", "Promedio", etc.
}