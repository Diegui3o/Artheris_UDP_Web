use serde::Serialize;
use crate::analysis::recommendations::RecommendationsReport;

/// Respuesta del endpoint de recomendaciones
#[derive(Debug, Clone, Serialize)]
pub struct RecommendationsResponse {
    pub flight_id: String,
    pub report: RecommendationsReport,
}