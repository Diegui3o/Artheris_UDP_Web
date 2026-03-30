use serde::Serialize;
use crate::analysis::correlation::CorrelationReport;

/// Respuesta del endpoint de correlaciones
#[derive(Debug, Clone, Serialize)]
pub struct CorrelationResponse {
    pub flight_id: String,
    pub report: CorrelationReport,
}