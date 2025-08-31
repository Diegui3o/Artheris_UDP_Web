use serde::Serialize;

/// Claves usadas para métricas de ángulos (ajústalas si tus nombres varían)
pub const FIELD_ROLL: &str = "AngleRoll";
pub const FIELD_PITCH: &str = "AnglePitch";
pub const FIELD_DES_ROLL: &str = "DesiredAngleRoll";
pub const FIELD_DES_PITCH: &str = "DesiredAnglePitch";

/// Campos que quieres graficar en la UI (preset para series)
pub const EXTRA_PLOT_FIELDS: &[&str] = &[
    "AccX", "AccY", "AccZ",
    "DesiredAnglePitch", "DesiredAngleRoll",
    "DesiredRateYaw",
    "g1", "g2",
    "k1", "k2",
    "m1", "m2",
    "tau_x", "tau_y", "tau_z",
];

/// Muestra “preprocesada” para el cálculo (ya en t_rel y con valores opcionales)
#[derive(Debug, Clone)]
pub struct AngleSample {
    /// tiempo relativo [s] desde el inicio del vuelo
    pub t_rel: f64,
    pub roll: Option<f64>,
    pub des_roll: Option<f64>,
    pub pitch: Option<f64>,
    pub des_pitch: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AngleMetrics {
    pub rmse_roll: Option<f64>,
    pub rmse_pitch: Option<f64>,
    pub itae_roll: Option<f64>,
    pub itae_pitch: Option<f64>,
    pub mae_roll: Option<f64>,
    pub mae_pitch: Option<f64>,
    pub n_segments_used: usize,
    pub duration_sec: f64,
}

pub fn compute_angle_metrics(samples: &[AngleSample]) -> AngleMetrics {
    if samples.len() < 2 {
        return AngleMetrics {
            rmse_roll: None,
            rmse_pitch: None,
            itae_roll: None,
            itae_pitch: None,
            mae_roll: None,
            mae_pitch: None,
            n_segments_used: 0,
            duration_sec: 0.0,
        };
    }

    let duration_sec = samples.last().unwrap().t_rel - samples.first().unwrap().t_rel;
    if duration_sec <= 0.0 {
        return AngleMetrics {
            rmse_roll: None,
            rmse_pitch: None,
            itae_roll: None,
            itae_pitch: None,
            mae_roll: None,
            mae_pitch: None,
            n_segments_used: 0,
            duration_sec: 0.0,
        };
    }

    let mut sum_abs_roll_dt = 0.0;
    let mut sum_abs_pitch_dt = 0.0;
    let mut sum_sq_roll_dt = 0.0;
    let mut sum_sq_pitch_dt = 0.0;
    let mut sum_itae_roll = 0.0;
    let mut sum_itae_pitch = 0.0;
    let mut used = 0usize;

    for w in samples.windows(2) {
        let a = &w[0];
        let b = &w[1];
        let dt = (b.t_rel - a.t_rel).max(0.0);
        if dt <= 0.0 {
            continue;
        }

        // Roll
        if let (Some(r_a), Some(dr_a), Some(r_b), Some(dr_b)) =
            (a.roll, a.des_roll, b.roll, b.des_roll)
        {
            let e_a = r_a - dr_a;
            let e_b = r_b - dr_b;

            // MAE / RMSE: integrar |e| y e² por rectángulo/trapecio simple
            // (usamos promedio entre e_a y e_b para suavizar)
            let abs_avg = 0.5 * (e_a.abs() + e_b.abs());
            let sq_avg = 0.5 * (e_a * e_a + e_b * e_b);
            sum_abs_roll_dt += abs_avg * dt;
            sum_sq_roll_dt += sq_avg * dt;

            // ITAE(t*|e|): trapecio sobre f(t)=t·|e|
            let f_a = a.t_rel * e_a.abs();
            let f_b = b.t_rel * e_b.abs();
            sum_itae_roll += 0.5 * (f_a + f_b) * dt;

            used += 1;
        }

        // Pitch
        if let (Some(p_a), Some(dp_a), Some(p_b), Some(dp_b)) =
            (a.pitch, a.des_pitch, b.pitch, b.des_pitch)
        {
            let e_a = p_a - dp_a;
            let e_b = p_b - dp_b;

            let abs_avg = 0.5 * (e_a.abs() + e_b.abs());
            let sq_avg = 0.5 * (e_a * e_a + e_b * e_b);
            sum_abs_pitch_dt += abs_avg * dt;
            sum_sq_pitch_dt += sq_avg * dt;

            let f_a = a.t_rel * e_a.abs();
            let f_b = b.t_rel * e_b.abs();
            sum_itae_pitch += 0.5 * (f_a + f_b) * dt;

            used += 1;
        }
    }

    let mae_roll = if used > 0 { Some(sum_abs_roll_dt / duration_sec) } else { None };
    let mae_pitch = if used > 0 { Some(sum_abs_pitch_dt / duration_sec) } else { None };
    let rmse_roll = if used > 0 { Some((sum_sq_roll_dt / duration_sec).sqrt()) } else { None };
    let rmse_pitch = if used > 0 { Some((sum_sq_pitch_dt / duration_sec).sqrt()) } else { None };
    let itae_roll = if used > 0 { Some(sum_itae_roll) } else { None };
    let itae_pitch = if used > 0 { Some(sum_itae_pitch) } else { None };

    AngleMetrics {
        rmse_roll,
        rmse_pitch,
        itae_roll,
        itae_pitch,
        mae_roll,
        mae_pitch,
        n_segments_used: used,
        duration_sec,
    }
}
