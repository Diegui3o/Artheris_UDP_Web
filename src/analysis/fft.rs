use rustfft::FftPlanner;
use num_complex::Complex;

pub fn compute_fft(signal: &[f64], sample_rate_hz: f64) -> (Vec<f64>, Vec<f64>) {
    let n = signal.len();
    let n_fft = n.next_power_of_two();
    
    // Preparar la señal con padding a potencia de 2
    let mut buffer: Vec<Complex<f64>> = vec![Complex::new(0.0, 0.0); n_fft];
    for i in 0..n {
        buffer[i] = Complex::new(signal[i], 0.0);
    }
    
    // Crear planificador FFT
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n_fft);
    
    // Ejecutar FFT
    fft.process(&mut buffer);
    
    // Calcular frecuencias y magnitudes (solo la mitad positiva)
    let nyquist = sample_rate_hz / 2.0;
    let freq_step = sample_rate_hz / n_fft as f64;
    
    let mut frequencies = Vec::new();
    let mut magnitudes = Vec::new();
    
    // Ignoramos la frecuencia 0 (DC) y tomamos hasta Nyquist
    for i in 1..(n_fft / 2) {
        let freq = i as f64 * freq_step;
        if freq <= nyquist {
            frequencies.push(freq);
            // Magnitud = sqrt(real^2 + imag^2) / n_fft
            let mag = buffer[i].norm() / n_fft as f64;
            magnitudes.push(mag);
        }
    }
    
    (frequencies, magnitudes)
}

/// Encuentra los picos dominantes en el espectro
/// 
/// # Arguments
/// * `frequencies` - Vector de frecuencias
/// * `magnitudes` - Vector de magnitudes
/// * `top_n` - Número de picos a encontrar
/// 
/// # Returns
/// * `Vec<(freq_hz, magnitude)>` - Lista de picos ordenados por magnitud
pub fn find_peaks(frequencies: &[f64], magnitudes: &[f64], top_n: usize) -> Vec<(f64, f64)> {
    if frequencies.len() < 3 {
        return Vec::new();
    }
    
    let mut peaks = Vec::new();
    
    // Buscar picos locales (mayor que vecinos)
    for i in 1..frequencies.len() - 1 {
        if magnitudes[i] > magnitudes[i - 1] && magnitudes[i] > magnitudes[i + 1] {
            peaks.push((frequencies[i], magnitudes[i]));
        }
    }
    
    // Ordenar por magnitud descendente
    peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    
    // Devolver los top_n picos
    peaks.into_iter().take(top_n).collect()
}

/// Estructura que almacena el espectro de una señal
#[derive(Debug, Clone, serde::Serialize)]
pub struct Spectrum {
    pub frequencies_hz: Vec<f64>,
    pub magnitudes: Vec<f64>,
    pub dominant_peaks: Vec<(f64, f64)>,  // (frecuencia, magnitud)
}

/// Calcula el espectro completo para una señal
pub fn compute_spectrum(
    signal: &[f64],
    sample_rate_hz: f64,
    top_peaks: usize,
) -> Spectrum {
    if signal.len() < 4 {
        return Spectrum {
            frequencies_hz: Vec::new(),
            magnitudes: Vec::new(),
            dominant_peaks: Vec::new(),
        };
    }
    
    let (freqs, mags) = compute_fft(signal, sample_rate_hz);
    let peaks = find_peaks(&freqs, &mags, top_peaks);
    
    Spectrum {
        frequencies_hz: freqs,
        magnitudes: mags,
        dominant_peaks: peaks,
    }
}