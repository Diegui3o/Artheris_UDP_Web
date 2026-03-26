use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperimentMetadata {
    // === Identificación ===
    pub experiment_id: String,        // Ej: "EXP_HOVER_001", "EXP_REPOSO_002"
    pub flight_id: String,            // UUID único para este vuelo (ya lo tienes)
    
    // === Temporal ===
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_seconds: Option<f32>,
    
    // === Configuración del dron ===
    pub sampling_rate_hz: u16,        // 25Hz, 50Hz, etc.
    pub esp32_loop_hz: u16,           // 1000Hz normalmente
    pub filter_type: String,           // "kalman", "complementary", "none"
    pub kalman_gains: Option<KalmanGains>,
    
    // === Condiciones de vuelo ===
    pub experiment_type: ExperimentType,  // Reposo, Hover, Agresivo, Secuencia
    pub description: Option<String>,
    
    // === Entorno ===
    pub location: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KalmanGains {
    pub k1: f32,
    pub k2: f32,
    pub k3: f32,
    pub g1: f32,
    pub g2: f32,
    pub g3: f32,
    pub m1: f32,
    pub m2: f32,
    pub m3: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExperimentType {
    #[serde(rename = "reposo")]
    Reposo,      // Dron quieto, sin motores
    #[serde(rename = "hover")]
    Hover,       // Vuelo estacionario
    #[serde(rename = "agresivo")]
    Agresivo,    // Maniobras bruscas
    #[serde(rename = "secuencia")]
    Secuencia,   // Secuencia predefinida
    #[serde(rename = "manual")]
    Manual,      // Vuelo manual libre
}

use std::fmt;

impl fmt::Display for ExperimentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            ExperimentType::Reposo => "reposo",
            ExperimentType::Hover => "hover",
            ExperimentType::Agresivo => "agresivo",
            ExperimentType::Secuencia => "secuencia",
            ExperimentType::Manual => "manual",
        };
        write!(f, "{}", s)
    }
}

impl Default for ExperimentMetadata {
    fn default() -> Self {
        Self {
            experiment_id: "UNKNOWN".to_string(),
            flight_id: uuid::Uuid::new_v4().to_string(),
            start_time: Utc::now(),
            end_time: None,
            duration_seconds: None,
            sampling_rate_hz: 25,
            esp32_loop_hz: 1000,
            filter_type: "kalman".to_string(),
            kalman_gains: None,
            experiment_type: ExperimentType::Manual,
            description: None,
            location: None,
            notes: None,
        }
    }
}