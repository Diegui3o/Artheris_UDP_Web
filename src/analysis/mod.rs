pub mod fft;
pub mod uncertainty;
pub mod anomaly;

// Re-exportar tipos de anomalías
pub use anomaly::{
    Anomaly, AnomalyReport, AnomalySummary, AnomalyType,
    detect_anomalies_in_signal, detect_noise_regions, analyze_flight_anomalies,
};