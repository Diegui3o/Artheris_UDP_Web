use serde::Serialize;
use crate::analysis::trend::TrendReport;

/// Respuesta del endpoint de tendencias
#[derive(Debug, Clone, Serialize)]
pub struct TrendResponse {
    pub flight_id: String,
    pub report: TrendReport,
}