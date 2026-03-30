use crate::config::metrics::AngleSample;
use serde::Serialize;

/// Señal extraída de un vuelo
#[derive(Debug, Clone)]
pub struct Signal {
    pub name: String,
    pub values: Vec<f64>,
}

/// Matriz de correlación entre señales
#[derive(Debug, Clone, Serialize)]
pub struct CorrelationMatrix {
    pub signals: Vec<String>,
    pub matrix: Vec<Vec<f64>>,
    pub pairs: Vec<CorrelationPair>,
}

/// Par de señales con su correlación
#[derive(Debug, Clone, Serialize)]
pub struct CorrelationPair {
    pub signal_a: String,
    pub signal_b: String,
    pub correlation: f64,
    pub strength: CorrelationStrength,
    pub interpretation: String,
}

/// Fuerza de la correlación
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum CorrelationStrength {
    #[serde(rename = "muy_fuerte")]
    MuyFuerte,      // > 0.9
    #[serde(rename = "fuerte")]
    Fuerte,         // > 0.7
    #[serde(rename = "moderada")]
    Moderada,       // > 0.5
    #[serde(rename = "debíl")]
    Debil,          // > 0.3
    #[serde(rename = "muy_debil")]
    MuyDebil,       // <= 0.3
    #[serde(rename = "negativa")]
    Negativa,       // < 0
}

/// Reporte completo de correlaciones
#[derive(Debug, Clone, Serialize)]
pub struct CorrelationReport {
    pub flight_id: String,
    pub matrix: CorrelationMatrix,
    pub strongest_correlations: Vec<CorrelationPair>,
    pub insights: Vec<String>,
}

/// Calcula el coeficiente de correlación de Pearson
pub fn pearson_correlation(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    if n < 3 {
        return 0.0;
    }
    
    let x_mean = x.iter().take(n).sum::<f64>() / n as f64;
    let y_mean = y.iter().take(n).sum::<f64>() / n as f64;
    
    let mut numerator = 0.0;
    let mut denom_x = 0.0;
    let mut denom_y = 0.0;
    
    for i in 0..n {
        let dx = x[i] - x_mean;
        let dy = y[i] - y_mean;
        numerator += dx * dy;
        denom_x += dx * dx;
        denom_y += dy * dy;
    }
    
    if denom_x == 0.0 || denom_y == 0.0 {
        return 0.0;
    }
    
    numerator / (denom_x.sqrt() * denom_y.sqrt())
}

/// Determina la fuerza de la correlación
pub fn correlation_strength(corr: f64) -> CorrelationStrength {
    let abs_corr = corr.abs();
    if corr < 0.0 {
        CorrelationStrength::Negativa
    } else if abs_corr > 0.9 {
        CorrelationStrength::MuyFuerte
    } else if abs_corr > 0.7 {
        CorrelationStrength::Fuerte
    } else if abs_corr > 0.5 {
        CorrelationStrength::Moderada
    } else if abs_corr > 0.3 {
        CorrelationStrength::Debil
    } else {
        CorrelationStrength::MuyDebil
    }
}

/// Genera interpretación de la correlación
pub fn interpret_correlation(signal_a: &str, signal_b: &str, corr: f64) -> String {
    let strength = correlation_strength(corr);
    
    match strength {
        CorrelationStrength::MuyFuerte => {
            format!("Correlación muy fuerte ({:.2}) entre {} y {}. Las señales se mueven prácticamente igual.", 
                    corr, signal_a, signal_b)
        }
        CorrelationStrength::Fuerte => {
            format!("Correlación fuerte ({:.2}) entre {} y {}. Relación significativa.", 
                    corr, signal_a, signal_b)
        }
        CorrelationStrength::Moderada => {
            format!("Correlación moderada ({:.2}) entre {} y {}. Existe relación notable.", 
                    corr, signal_a, signal_b)
        }
        CorrelationStrength::Debil => {
            format!("Correlación débil ({:.2}) entre {} y {}. Relación tenue.", 
                    corr, signal_a, signal_b)
        }
        CorrelationStrength::MuyDebil => {
            format!("Correlación muy débil ({:.2}) entre {} y {}. Prácticamente independientes.", 
                    corr, signal_a, signal_b)
        }
        CorrelationStrength::Negativa => {
            format!("Correlación negativa ({:.2}) entre {} y {}. Una aumenta cuando la otra disminuye.", 
                    corr, signal_a, signal_b)
        }
    }
}

/// Extrae señales de las muestras
pub fn extract_signals(samples: &[AngleSample]) -> Vec<Signal> {
    let mut signals = Vec::new();
    
    // 1. Error de roll
    let roll_errors: Vec<f64> = samples.iter()
        .filter_map(|s| {
            if let (Some(ref_val), Some(kalman)) = (s.des_roll, s.kalman_roll) {
                Some(ref_val - kalman)
            } else {
                None
            }
        })
        .collect();
    
    if !roll_errors.is_empty() {
        signals.push(Signal {
            name: "Error Roll".to_string(),
            values: roll_errors,
        });
    }
    
    // 2. Error de pitch
    let pitch_errors: Vec<f64> = samples.iter()
        .filter_map(|s| {
            if let (Some(ref_val), Some(kalman)) = (s.des_pitch, s.kalman_pitch) {
                Some(ref_val - kalman)
            } else {
                None
            }
        })
        .collect();
    
    if !pitch_errors.is_empty() {
        signals.push(Signal {
            name: "Error Pitch".to_string(),
            values: pitch_errors,
        });
    }
    
    // 3. Raw roll (crudo)
    let raw_roll: Vec<f64> = samples.iter()
        .filter_map(|s| s.roll)
        .collect();
    
    if !raw_roll.is_empty() {
        signals.push(Signal {
            name: "Raw Roll".to_string(),
            values: raw_roll,
        });
    }
    
    // 4. Raw pitch
    let raw_pitch: Vec<f64> = samples.iter()
        .filter_map(|s| s.pitch)
        .collect();
    
    if !raw_pitch.is_empty() {
        signals.push(Signal {
            name: "Raw Pitch".to_string(),
            values: raw_pitch,
        });
    }
    
    // 5. Kalman roll
    let kalman_roll: Vec<f64> = samples.iter()
        .filter_map(|s| s.kalman_roll)
        .collect();
    
    if !kalman_roll.is_empty() {
        signals.push(Signal {
            name: "Kalman Roll".to_string(),
            values: kalman_roll,
        });
    }
    
    // 6. Kalman pitch
    let kalman_pitch: Vec<f64> = samples.iter()
        .filter_map(|s| s.kalman_pitch)
        .collect();
    
    if !kalman_pitch.is_empty() {
        signals.push(Signal {
            name: "Kalman Pitch".to_string(),
            values: kalman_pitch,
        });
    }
    
    signals
}

/// Calcula la matriz de correlación completa
pub fn compute_correlation_matrix(signals: &[Signal]) -> CorrelationMatrix {
    let signal_names: Vec<String> = signals.iter().map(|s| s.name.clone()).collect();
    let n = signals.len();
    
    let mut matrix = vec![vec![0.0; n]; n];
    let mut pairs = Vec::new();
    
    for i in 0..n {
        for j in 0..n {
            let corr = if i == j {
                1.0
            } else {
                pearson_correlation(&signals[i].values, &signals[j].values)
            };
            matrix[i][j] = corr;
            
            // Solo guardar pares únicos (i < j)
            if i < j {
                let strength = correlation_strength(corr);
                pairs.push(CorrelationPair {
                    signal_a: signal_names[i].clone(),
                    signal_b: signal_names[j].clone(),
                    correlation: corr,
                    strength: strength.clone(),
                    interpretation: interpret_correlation(&signal_names[i], &signal_names[j], corr),
                });
            }
        }
    }
    
    // Ordenar pares por correlación (mayor a menor)
    pairs.sort_by(|a, b| b.correlation.partial_cmp(&a.correlation).unwrap());
    
    CorrelationMatrix {
        signals: signal_names,
        matrix,
        pairs,
    }
}

/// Analiza un vuelo completo para correlaciones
pub fn analyze_correlations(
    flight_id: &str,
    samples: &[AngleSample],
) -> CorrelationReport {
    let signals = extract_signals(samples);
    
    if signals.len() < 2 {
        return CorrelationReport {
            flight_id: flight_id.to_string(),
            matrix: CorrelationMatrix {
                signals: Vec::new(),
                matrix: Vec::new(),
                pairs: Vec::new(),
            },
            strongest_correlations: Vec::new(),
            insights: vec!["No hay suficientes señales para calcular correlaciones.".to_string()],
        };
    }
    
    let matrix = compute_correlation_matrix(&signals);
    
    // Top 5 correlaciones más fuertes
    let strongest = matrix.pairs.iter()
        .take(5)
        .cloned()
        .collect();
    
    // Generar insights automáticos
    let mut insights = Vec::new();
    
    for pair in &matrix.pairs {
        match pair.strength {
            CorrelationStrength::MuyFuerte => {
                insights.push(format!(
                    "⚠️ {} y {} tienen correlación muy fuerte ({:.2}). Podría indicar redundancia o relación causal directa.",
                    pair.signal_a, pair.signal_b, pair.correlation
                ));
            }
            CorrelationStrength::Fuerte => {
                if pair.signal_a.contains("Error") && pair.signal_b.contains("Motor") {
                    insights.push(format!(
                        "🔍 El error de seguimiento ({}) está fuertemente correlacionado con {} ({:.2}). Posible vibración inducida por motor.",
                        pair.signal_a, pair.signal_b, pair.correlation
                    ));
                }
            }
            CorrelationStrength::Moderada => {
                if pair.signal_a.contains("Raw") && pair.signal_b.contains("Kalman") {
                    insights.push(format!(
                        "📊 {} y {} tienen correlación moderada ({:.2}). El filtro Kalman está suavizando pero manteniendo la tendencia.",
                        pair.signal_a, pair.signal_b, pair.correlation
                    ));
                }
            }
            _ => {}
        }
    }
    
    if insights.is_empty() {
        insights.push("✅ No se detectaron correlaciones significativas. Las señales son independientes.".to_string());
    }
    
    CorrelationReport {
        flight_id: flight_id.to_string(),
        matrix,
        strongest_correlations: strongest,
        insights,
    }
}