use rand::distributions::Uniform;
use rand_distr::{Normal, Distribution};
use serde::Serialize;

/// Distribución estadística de una fuente de error
#[derive(Debug, Clone, Serialize)]
pub enum DistributionType {
    Normal { mean: f64, std_dev: f64 },
    Uniform { min: f64, max: f64 },
}

/// Una fuente de error individual en el presupuesto de incertidumbre
#[derive(Debug, Clone, Serialize)]
pub struct UncertaintySource {
    pub name: String,
    pub value: f64,           // Valor de la incertidumbre (desviación estándar)
    pub distribution: DistributionType,
    pub description: String,
}

/// Presupuesto completo de incertidumbre
#[derive(Debug, Clone, Serialize)]
pub struct UncertaintyBudget {
    pub sources: Vec<UncertaintySource>,
    pub standard_uncertainty: f64,     // u_c (k=1) combinada por RSS
    pub expanded_uncertainty_k2: f64,  // U (k=2, 95% confianza)
    pub coverage_factor: f64,          // k = 2
    pub confidence_percent: f64,       // 95.45%
}

/// Resultado de simulación Monte Carlo
#[derive(Debug, Clone, Serialize)]
pub struct MonteCarloResult {
    pub iterations: usize,
    pub mean_error: f64,
    pub std_dev_error: f64,
    pub percentiles: Percentiles,
    pub histogram_bins: Vec<HistogramBin>,
}

/// Percentiles del error
#[derive(Debug, Clone, Serialize)]
pub struct Percentiles {
    pub p5: f64,   // 5% de los errores están por debajo
    pub p25: f64,
    pub p50: f64,  // mediana
    pub p75: f64,
    pub p95: f64,
}

/// Bin del histograma para visualización
#[derive(Debug, Clone, Serialize)]
pub struct HistogramBin {
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub count: usize,
}

/// Reporte completo de incertidumbre para un vuelo
#[derive(Debug, Clone, Serialize)]
pub struct UncertaintyReport {
    pub flight_id: String,
    pub budget: UncertaintyBudget,
    pub monte_carlo: MonteCarloResult,
    pub validation: ValidationResult,
}

/// Resultado de validación (error observado vs incertidumbre calculada)
#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult {
    pub observed_error_rms: f64,
    pub within_interval: bool,
    pub interval_lower: f64,
    pub interval_upper: f64,
}

// ==================== FUNCIONES DE CÁLCULO ====================

/// Combina incertidumbres por Root Sum Square (RSS) - Método GUM
pub fn combine_rss(sources: &[UncertaintySource]) -> f64 {
    sources.iter()
        .map(|s| s.value * s.value)
        .sum::<f64>()
        .sqrt()
}

/// Calcula incertidumbre expandida con factor de cobertura k
pub fn expanded_uncertainty(standard: f64, k: f64) -> f64 {
    standard * k
}

/// Simulación Monte Carlo
pub fn monte_carlo_simulation(
    sources: &[UncertaintySource],
    iterations: usize,
) -> MonteCarloResult {
    let mut rng = rand::thread_rng();
    let mut errors = Vec::with_capacity(iterations);
    
    for _ in 0..iterations {
        let mut total_error = 0.0;
        
        for source in sources {
            let sample = match &source.distribution {
                DistributionType::Normal { mean, std_dev } => {
                    let normal = Normal::new(*mean, *std_dev).unwrap();
                    normal.sample(&mut rng)
                }
                DistributionType::Uniform { min, max } => {
                    let uniform = Uniform::new_inclusive(*min, *max);
                    uniform.sample(&mut rng)
                }
            };
            total_error += sample;
        }
        errors.push(total_error);
    }
    
    // Ordenar para percentiles
    errors.sort_by(|a, b| a.partial_cmp(b).unwrap());
    
    let mean_error = errors.iter().sum::<f64>() / iterations as f64;
    let variance = errors.iter().map(|e| (e - mean_error).powi(2)).sum::<f64>() / iterations as f64;
    let std_dev_error = variance.sqrt();
    
    let percentile = |p: f64| -> f64 {
        let idx = ((p / 100.0) * iterations as f64) as usize;
        errors.get(idx).copied().unwrap_or(0.0)
    };
    
    // Crear histograma con 20 bins
    let min_error = errors.first().copied().unwrap_or(0.0);
    let max_error = errors.last().copied().unwrap_or(0.0);
    let bin_width = (max_error - min_error) / 20.0;
    let mut histogram_bins = Vec::new();
    
    for i in 0..20 {
        let lower = min_error + i as f64 * bin_width;
        let upper = lower + bin_width;
        let count = errors.iter()
            .filter(|&e| *e >= lower && *e < upper)
            .count();
        histogram_bins.push(HistogramBin { lower_bound: lower, upper_bound: upper, count });
    }
    
    MonteCarloResult {
        iterations,
        mean_error,
        std_dev_error,
        percentiles: Percentiles {
            p5: percentile(5.0),
            p25: percentile(25.0),
            p50: percentile(50.0),
            p75: percentile(75.0),
            p95: percentile(95.0),
        },
        histogram_bins,
    }
}

/// Crea un presupuesto de incertidumbre a partir de fuentes
pub fn create_uncertainty_budget(sources: Vec<UncertaintySource>) -> UncertaintyBudget {
    let standard = combine_rss(&sources);
    let expanded = expanded_uncertainty(standard, 2.0);
    
    UncertaintyBudget {
        sources,
        standard_uncertainty: standard,
        expanded_uncertainty_k2: expanded,
        coverage_factor: 2.0,
        confidence_percent: 95.45,
    }
}