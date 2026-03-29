use serde::Serialize;
use crate::analysis::uncertainty::{UncertaintyBudget, MonteCarloResult, ValidationResult};

/// Respuesta del endpoint de incertidumbre
#[derive(Debug, Clone, Serialize)]
pub struct UncertaintyResponse {
    pub flight_id: String,
    pub budget: UncertaintyBudget,
    pub monte_carlo: MonteCarloResult,
    pub validation: ValidationResult,
}