use crate::config::historical_types::*;
use crate::config::metrics::FullFlightMetrics;
use crate::ws_server::questdb::OptionalDb;
use anyhow::Result;
use std::collections::HashMap;

/// Guarda las métricas de un vuelo en la tabla histórica
pub async fn store_flight_metrics(
    questdb: &OptionalDb,
    flight_id: &str,
    metrics: &FullFlightMetrics,
) -> Result<()> {
    println!("---! store_flight_metrics llamado para flight: {}", flight_id);
    
    let db = questdb.inner.lock().await;
    if let Some(qdb) = db.as_ref() {
        qdb.insert_historical_metrics(flight_id, metrics).await?;
        println!("---! ✅ Métricas guardadas exitosamente");
    } else {
        println!("---X No hay conexión a QuestDB");
    }
    Ok(())
}

/// Obtiene todas las métricas históricas para un tipo de vuelo
pub async fn get_historical_metrics(
    questdb: &OptionalDb,
    flight_type: &str,
) -> Result<Vec<FullFlightMetrics>> {
    let db = questdb.inner.lock().await;
    if let Some(qdb) = db.as_ref() {
        qdb.fetch_historical_metrics(flight_type).await
    } else {
        Ok(Vec::new())
    }
}

/// Calcula estadísticas históricas a partir de una lista de métricas
pub fn compute_historical_stats(
    metrics_list: &[FullFlightMetrics],
    flight_type: &str,
) -> HistoricalStats {
    let mut rmse_roll_vals = Vec::new();
    let mut rmse_pitch_vals = Vec::new();
    let mut improvement_vals = Vec::new();
    let mut variance_roll_vals = Vec::new();
    let mut variance_pitch_vals = Vec::new();
    
    for m in metrics_list {
        if let Some(v) = m.error_metrics.rmse_roll {
            rmse_roll_vals.push(v);
        }
        if let Some(v) = m.error_metrics.rmse_pitch {
            rmse_pitch_vals.push(v);
        }
        if let Some(v) = m.comparison_roll.improvement_percent {
            improvement_vals.push(v);
        }
        if let Some(v) = m.error_metrics.variance_roll {
            variance_roll_vals.push(v);
        }
        if let Some(v) = m.error_metrics.variance_pitch {
            variance_pitch_vals.push(v);
        }
    }
    
    let stats = |vals: &[f64]| -> MetricStats {
        if vals.is_empty() {
            return MetricStats {
                mean: 0.0,
                std_dev: 0.0,
                min: 0.0,
                max: 0.0,
                median: 0.0,
            };
        }
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        let variance = vals.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / vals.len() as f64;
        let std_dev = variance.sqrt();
        let min = *vals.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
        let max = *vals.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap();
        
        let mut sorted = vals.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];
        
        MetricStats { mean, std_dev, min, max, median }
    };
    
    let percentiles = |vals: &[f64]| -> PercentileStats {
        if vals.is_empty() {
            return PercentileStats { p5: 0.0, p25: 0.0, p50: 0.0, p75: 0.0, p95: 0.0 };
        }
        let mut sorted = vals.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let len = sorted.len();
        let p5 = sorted[(len as f64 * 0.05) as usize];
        let p25 = sorted[(len as f64 * 0.25) as usize];
        let p50 = sorted[(len as f64 * 0.50) as usize];
        let p75 = sorted[(len as f64 * 0.75) as usize];
        let p95 = sorted[(len as f64 * 0.95) as usize];
        PercentileStats { p5, p25, p50, p75, p95 }
    };
    
    let metrics_map = HashMap::from([
        ("rmse_roll".to_string(), stats(&rmse_roll_vals)),
        ("rmse_pitch".to_string(), stats(&rmse_pitch_vals)),
        ("improvement_percent".to_string(), stats(&improvement_vals)),
        ("variance_roll".to_string(), stats(&variance_roll_vals)),
        ("variance_pitch".to_string(), stats(&variance_pitch_vals)),
    ]);
    
    HistoricalStats {
        flight_type: flight_type.to_string(),
        sample_count: metrics_list.len(),
        metrics: metrics_map,
        percentiles: percentiles(&rmse_roll_vals),
        last_updated: chrono::Utc::now(),
    }
}

/// Compara un vuelo actual con el histórico
pub fn compare_flight_with_historical(
    current: &FullFlightMetrics,
    historical: &[FullFlightMetrics],
) -> HistoricalComparison {
    let flight_type = match &current.flight_type {
        crate::config::metrics::FlightType::Reposo => "reposo",
        crate::config::metrics::FlightType::Hover => "hover",
        crate::config::metrics::FlightType::Maniobra => "maniobra",
        crate::config::metrics::FlightType::Desconocido => "desconocido",
    };
    
    let stats = compute_historical_stats(historical, flight_type);
    
    // Helper para comparar una métrica
    let compare_metric = |value: Option<f64>, name: &str| -> Option<MetricComparison> {
        let v = value?;
        let metric_stats = stats.metrics.get(name)?;
        let mean = metric_stats.mean;
        let std_dev = if metric_stats.std_dev > 0.0 { metric_stats.std_dev } else { 1.0 };
        let z_score = (v - mean) / std_dev;
        
        let mut all_vals: Vec<f64> = historical.iter()
            .filter_map(|m| {
                match name {
                    "rmse_roll" => m.error_metrics.rmse_roll,
                    "rmse_pitch" => m.error_metrics.rmse_pitch,
                    "improvement_percent" => m.comparison_roll.improvement_percent,
                    "variance_roll" => m.error_metrics.variance_roll,
                    "variance_pitch" => m.error_metrics.variance_pitch,
                    _ => None,
                }
            })
            .collect();
        all_vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let count_better = all_vals.iter().filter(|&&x| x < v).count();
        let percentile = if all_vals.is_empty() { 50.0 } else { (count_better as f64 / all_vals.len() as f64) * 100.0 };
        
        let better_than_average = if name == "improvement_percent" {
            v > mean
        } else {
            v < mean
        };
        
        let interpretation = if better_than_average {
            if percentile >= 90.0 {
                "Excelente: superior al 90% de vuelos similares".to_string()
            } else if percentile >= 75.0 {
                "Muy bueno: superior al 75% de vuelos similares".to_string()
            } else if percentile >= 50.0 {
                "Bueno: mejor que la media".to_string()
            } else {
                "Aceptable: dentro del rango normal".to_string()
            }
        } else {
            if percentile <= 10.0 {
                "Necesita mejora: en el peor 10% de vuelos".to_string()
            } else if percentile <= 25.0 {
                "Por debajo del promedio: considerar ajustes".to_string()
            } else {
                "Dentro del rango normal, pero puede mejorar".to_string()
            }
        };
        
        Some(MetricComparison {
            value: v,
            mean,
            std_dev: metric_stats.std_dev,
            z_score,
            percentile,
            better_than_average,
            interpretation,
        })
    };
    
    let current_metrics = FlightComparisonMetrics {
        rmse_roll: current.error_metrics.rmse_roll,
        rmse_pitch: current.error_metrics.rmse_pitch,
        improvement_percent: current.comparison_roll.improvement_percent,
        variance_roll: current.error_metrics.variance_roll,
        variance_pitch: current.error_metrics.variance_pitch,
        itae_roll: current.error_metrics.itae_roll,
        itae_pitch: current.error_metrics.itae_pitch,
    };
    
    let details = ComparisonDetails {
        rmse_roll: compare_metric(current_metrics.rmse_roll, "rmse_roll").unwrap_or_else(|| MetricComparison {
            value: 0.0, mean: 0.0, std_dev: 0.0, z_score: 0.0, percentile: 0.0,
            better_than_average: false, interpretation: "Datos insuficientes".to_string(),
        }),
        rmse_pitch: compare_metric(current_metrics.rmse_pitch, "rmse_pitch").unwrap_or_else(|| MetricComparison {
            value: 0.0, mean: 0.0, std_dev: 0.0, z_score: 0.0, percentile: 0.0,
            better_than_average: false, interpretation: "Datos insuficientes".to_string(),
        }),
        improvement_percent: compare_metric(current_metrics.improvement_percent, "improvement_percent").unwrap_or_else(|| MetricComparison {
            value: 0.0, mean: 0.0, std_dev: 0.0, z_score: 0.0, percentile: 0.0,
            better_than_average: false, interpretation: "Datos insuficientes".to_string(),
        }),
        variance_roll: compare_metric(current_metrics.variance_roll, "variance_roll").unwrap_or_else(|| MetricComparison {
            value: 0.0, mean: 0.0, std_dev: 0.0, z_score: 0.0, percentile: 0.0,
            better_than_average: false, interpretation: "Datos insuficientes".to_string(),
        }),
        variance_pitch: compare_metric(current_metrics.variance_pitch, "variance_pitch").unwrap_or_else(|| MetricComparison {
            value: 0.0, mean: 0.0, std_dev: 0.0, z_score: 0.0, percentile: 0.0,
            better_than_average: false, interpretation: "Datos insuficientes".to_string(),
        }),
    };
    
    // Calcular puntuación de calidad (0-100)
    let mut quality_score: f64 = 0.0;
    let mut count: f64 = 0.0;
    
    if let Some(v) = current_metrics.rmse_roll {
        let stats = stats.metrics.get("rmse_roll").unwrap();
        if stats.std_dev > 0.0 {
            let normalized = (v - stats.min) / (stats.max - stats.min);
            let score: f64 = (1.0 - normalized) * 50.0;
            quality_score += score.clamp(0.0, 50.0);
            count += 1.0;
        }
    }
    
    if let Some(v) = current_metrics.improvement_percent {
        let stats = stats.metrics.get("improvement_percent").unwrap();
        if stats.std_dev > 0.0 {
            let normalized = (v - stats.min) / (stats.max - stats.min);
            let score: f64 = normalized * 30.0;
            quality_score += score.clamp(0.0, 30.0);
            count += 1.0;
        }
    }
    
    if let Some(v) = current_metrics.variance_roll {
        let stats = stats.metrics.get("variance_roll").unwrap();
        if stats.std_dev > 0.0 {
            let normalized = (v - stats.min) / (stats.max - stats.min);
            let score: f64 = (1.0 - normalized) * 20.0;
            quality_score += score.clamp(0.0, 20.0);
            count += 1.0;
        }
    }
    
    if count > 0.0 {
        quality_score = quality_score / count;
    } else {
        quality_score = 50.0;
    }
    
    // Calcular ranking
    let mut all_rmse: Vec<f64> = historical.iter()
        .filter_map(|m| m.error_metrics.rmse_roll)
        .collect();
    all_rmse.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let current_rmse = current.error_metrics.rmse_roll.unwrap_or(f64::INFINITY);
    let position = all_rmse.iter().position(|&x| x > current_rmse).unwrap_or(all_rmse.len()) + 1;
    let percentile_rank = if all_rmse.is_empty() { 50.0 } else { (position as f64 / all_rmse.len() as f64) * 100.0 };
    let label = if percentile_rank <= 10.0 {
        "Excelente".to_string()
    } else if percentile_rank <= 25.0 {
        "Muy bueno".to_string()
    } else if percentile_rank <= 50.0 {
        "Bueno".to_string()
    } else if percentile_rank <= 75.0 {
        "Promedio".to_string()
    } else {
        "Requiere mejora".to_string()
    };
    
    let rank = RankInfo {
        position,
        total: all_rmse.len(),
        percentile: percentile_rank,
        label,
    };
    
    HistoricalComparison {
        flight_id: current.flight_id.clone(),
        flight_type: flight_type.to_string(),
        current: current_metrics,
        historical: stats,
        comparison: details,
        quality_score: quality_score.round(),
        rank,
    }
}