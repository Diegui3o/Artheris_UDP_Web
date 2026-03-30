use serde::Serialize;
use crate::analysis::score::QualityScore;

/// Respuesta del endpoint de score de calidad
#[derive(Debug, Clone, Serialize)]
pub struct ScoreResponse {
    pub flight_id: String,
    pub score: QualityScore,
}