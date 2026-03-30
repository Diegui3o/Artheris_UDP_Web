use serde::Serialize;
use crate::analysis::anomaly::{AnomalyReport, AnomalySummary, Anomaly};

/// Respuesta del endpoint de anomalías
#[derive(Debug, Clone, Serialize)]
pub struct AnomalyResponse {
    pub flight_id: String,
    pub report: AnomalyReport,
}