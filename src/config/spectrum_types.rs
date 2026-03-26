use serde::Serialize;

/// Espectro de una señal
#[derive(Debug, Clone, Serialize)]
pub struct Spectrum {
    pub frequencies_hz: Vec<f64>,
    pub magnitudes: Vec<f64>,
    pub dominant_peaks: Vec<Peak>,
}

/// Pico de frecuencia dominante
#[derive(Debug, Clone, Serialize)]
pub struct Peak {
    pub frequency_hz: f64,
    pub magnitude: f64,
}

/// Espectro completo de un vuelo
#[derive(Debug, Clone, Serialize)]
pub struct FlightSpectrum {
    pub flight_id: String,
    pub sample_rate_hz: f64,
    pub sample_count: usize,
    
    /// Espectro del error (phi_ref - KalmanAngleRoll)
    pub error_spectrum: Spectrum,
    
    /// Espectro promedio de los motores
    pub motors_spectrum: Spectrum,
    
    /// Espectro del acelerómetro X
    pub acc_x_spectrum: Spectrum,
    
    /// Espectro del acelerómetro Y
    pub acc_y_spectrum: Spectrum,
    
    /// Espectro del acelerómetro Z
    pub acc_z_spectrum: Spectrum,
    
    /// Correlaciones encontradas entre frecuencias
    pub correlations: Vec<Correlation>,
}

/// Correlación entre frecuencias de diferentes señales
#[derive(Debug, Clone, Serialize)]
pub struct Correlation {
    pub frequency_hz: f64,
    pub sources: Vec<String>,
    pub description: String,
}