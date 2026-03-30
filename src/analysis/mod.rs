pub mod fft;
pub mod uncertainty;
pub mod anomaly;
pub mod correlation;
pub mod trend;
pub mod recommendations;
pub mod score;

pub use anomaly::{
    Anomaly, AnomalyReport, AnomalySummary, AnomalyType,
    detect_anomalies_in_signal, detect_noise_regions, analyze_flight_anomalies,
};

pub use correlation::{
    CorrelationMatrix, CorrelationPair, CorrelationReport, CorrelationStrength,
    pearson_correlation, correlation_strength, interpret_correlation,
    extract_signals, compute_correlation_matrix, analyze_correlations,
};

pub use trend::{
    TrendPrediction, TrendDirection, TrendReport,
    compute_trend, analyze_trends,
};

pub use recommendations::{
    Recommendation, RecommendationsReport, Priority, Area,
    generate_recommendations,
};

pub use score::{
    QualityScore, QualityCategory, ScoreBreakdown,
    compute_quality_score, quick_score,
};