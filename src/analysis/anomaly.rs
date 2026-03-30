use serde::Serialize;

/// Tipos de anomalías detectadas
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum AnomalyType {
    /// Pico aislado (glitch)
    Spike,
    /// Ruido excesivo en ventana
    ExcessiveNoise,
    /// Pérdida de señal (valores constantes)
    SignalLoss,
    /// Deriva anormal
    AbnormalDrift,
    /// Oscilación no deseada
    Oscillation,
}

/// Una anomalía detectada
#[derive(Debug, Clone, Serialize)]
pub struct Anomaly {
    pub timestamp: f64,           // Tiempo en segundos desde inicio del vuelo
    pub anomaly_type: AnomalyType,
    pub severity: f64,            // 0-100, qué tan grave es
    pub description: String,
    pub affected_axis: String,    // "roll", "pitch", "yaw"
    pub value: Option<f64>,       // Valor en el momento de la anomalía
}

/// Resultado del análisis de anomalías
#[derive(Debug, Clone, Serialize)]
pub struct AnomalyReport {
    pub flight_id: String,
    pub total_anomalies: usize,
    pub anomalies: Vec<Anomaly>,
    pub summary: AnomalySummary,
}

/// Resumen estadístico de anomalías
#[derive(Debug, Clone, Serialize)]
pub struct AnomalySummary {
    pub spikes_count: usize,
    pub noise_count: usize,
    pub signal_loss_count: usize,
    pub drift_count: usize,
    pub oscillation_count: usize,
    pub max_severity: f64,
    pub overall_quality_score: u8,  // 0-100
}

/// Detecta anomalías en una señal
pub fn detect_anomalies_in_signal(
    signal: &[(f64, f64)], // (timestamp, value)
    axis: &str,
    threshold_multiplier: f64,
) -> Vec<Anomaly> {
    let mut anomalies = Vec::new();
    
    if signal.len() < 5 {
        return anomalies;
    }
    
    // Extraer solo valores
    let values: Vec<f64> = signal.iter().map(|(_, v)| *v).collect();
    let timestamps: Vec<f64> = signal.iter().map(|(t, _)| *t).collect();
    
    // 1. DETECCIÓN DE PICOS (SPIKES)
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    let std_dev = variance.sqrt();
    let spike_threshold = threshold_multiplier * std_dev.max(0.1);
    
    for i in 1..values.len() - 1 {
        let prev = values[i - 1];
        let curr = values[i];
        let next = values[i + 1];
        
        // Pico aislado: valor actual es muy diferente de vecinos
        let diff_prev = (curr - prev).abs();
        let diff_next = (curr - next).abs();
        
        if diff_prev > spike_threshold && diff_next > spike_threshold {
            let severity = ((diff_prev + diff_next) / (2.0 * spike_threshold) * 100.0).min(100.0);
            
            anomalies.push(Anomaly {
                timestamp: timestamps[i],
                anomaly_type: AnomalyType::Spike,
                severity,
                description: format!("Pico aislado de {:.3}° en {}", curr, axis),
                affected_axis: axis.to_string(),
                value: Some(curr),
            });
        }
    }
    
    // 2. DETECCIÓN DE PÉRDIDA DE SEÑAL
    let mut constant_count = 0;
    let constant_threshold = 0.01; // 0.01° de cambio
    
    for i in 1..values.len() {
        if (values[i] - values[i - 1]).abs() < constant_threshold {
            constant_count += 1;
        } else {
            if constant_count > 10 {
                let severity = (constant_count as f64 / 50.0 * 100.0).min(100.0);
                anomalies.push(Anomaly {
                    timestamp: timestamps[i - constant_count],
                    anomaly_type: AnomalyType::SignalLoss,
                    severity,
                    description: format!("Señal constante por {} muestras en {}", constant_count, axis),
                    affected_axis: axis.to_string(),
                    value: Some(values[i - 1]),
                });
            }
            constant_count = 0;
        }
    }
    
    // 3. DETECCIÓN DE OSCILACIONES
    let mut zero_crossings = 0;
    let mut last_sign = 0.0;
    
    for i in 1..values.len() {
        let diff = values[i] - values[i - 1];
        let sign = diff.signum();
        
        if last_sign != 0.0 && sign != 0.0 && sign != last_sign {
            zero_crossings += 1;
        }
        last_sign = sign;
    }
    
    let oscillation_freq = zero_crossings as f64 / (timestamps.last().unwrap() - timestamps[0]);
    
    if oscillation_freq > 2.0 && oscillation_freq < 15.0 {
        let severity = (oscillation_freq / 10.0 * 100.0).min(100.0);
        anomalies.push(Anomaly {
            timestamp: timestamps[0],
            anomaly_type: AnomalyType::Oscillation,
            severity,
            description: format!("Oscilación detectada a {:.1} Hz en {}", oscillation_freq, axis),
            affected_axis: axis.to_string(),
            value: None,
        });
    }
    
    anomalies
}

/// Calcula el ruido en ventanas deslizantes
pub fn detect_noise_regions(
    signal: &[(f64, f64)],
    axis: &str,
    window_size: usize,
    noise_threshold: f64,
) -> Vec<Anomaly> {
    let mut anomalies = Vec::new();
    
    if signal.len() < window_size {
        return anomalies;
    }
    
    for i in 0..signal.len() - window_size {
        let window: Vec<f64> = signal[i..i + window_size].iter().map(|(_, v)| *v).collect();
        
        let mean = window.iter().sum::<f64>() / window.len() as f64;
        let variance = window.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / window.len() as f64;
        let std_dev = variance.sqrt();
        
        if std_dev > noise_threshold {
            let severity = (std_dev / noise_threshold * 100.0).min(100.0);
            anomalies.push(Anomaly {
                timestamp: signal[i].0,
                anomaly_type: AnomalyType::ExcessiveNoise,
                severity,
                description: format!("Ruido excesivo (σ={:.3}°) en {} desde t={:.1}s", std_dev, axis, signal[i].0),
                affected_axis: axis.to_string(),
                value: Some(std_dev),
            });
        }
    }
    
    anomalies
}

/// Analiza un vuelo completo para detectar anomalías
pub fn analyze_flight_anomalies(
    flight_id: &str,
    roll_errors: &[(f64, f64)],      // (timestamp, error)
    pitch_errors: &[(f64, f64)],
    raw_roll: &[(f64, f64)],
    raw_pitch: &[(f64, f64)],
) -> AnomalyReport {
    let mut all_anomalies = Vec::new();
    
    // Detectar en errores
    let roll_spikes = detect_anomalies_in_signal(roll_errors, "roll_error", 3.0);
    let pitch_spikes = detect_anomalies_in_signal(pitch_errors, "pitch_error", 3.0);
    
    // Detectar ruido
    let roll_noise = detect_noise_regions(roll_errors, "roll_error", 10, 0.3);
    let pitch_noise = detect_noise_regions(pitch_errors, "pitch_error", 10, 0.3);
    
    // Detectar en raw (ruido del sensor)
    let raw_roll_noise = detect_noise_regions(raw_roll, "raw_roll", 10, 0.5);
    let raw_pitch_noise = detect_noise_regions(raw_pitch, "raw_pitch", 10, 0.5);
    
    all_anomalies.extend(roll_spikes);
    all_anomalies.extend(pitch_spikes);
    all_anomalies.extend(roll_noise);
    all_anomalies.extend(pitch_noise);
    all_anomalies.extend(raw_roll_noise);
    all_anomalies.extend(raw_pitch_noise);
    
    // Ordenar por timestamp
    all_anomalies.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
    
    // Resumen
    let mut summary = AnomalySummary {
        spikes_count: 0,
        noise_count: 0,
        signal_loss_count: 0,
        drift_count: 0,
        oscillation_count: 0,
        max_severity: 0.0,
        overall_quality_score: 100,
    };
    
    for a in &all_anomalies {
        match a.anomaly_type {
            AnomalyType::Spike => summary.spikes_count += 1,
            AnomalyType::ExcessiveNoise => summary.noise_count += 1,
            AnomalyType::SignalLoss => summary.signal_loss_count += 1,
            AnomalyType::AbnormalDrift => summary.drift_count += 1,
            AnomalyType::Oscillation => summary.oscillation_count += 1,
        }
        summary.max_severity = summary.max_severity.max(a.severity);
    }
    
    // Calcular score de calidad (0-100)
    let total_penalty = (summary.spikes_count * 5) 
        + (summary.noise_count * 10)
        + (summary.signal_loss_count * 20)
        + (summary.oscillation_count * 15);
    
    summary.overall_quality_score = (100 - total_penalty.min(100)) as u8;
    
    AnomalyReport {
        flight_id: flight_id.to_string(),
        total_anomalies: all_anomalies.len(),
        anomalies: all_anomalies,
        summary,
    }
}