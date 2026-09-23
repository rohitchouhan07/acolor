use rustfft::{FftPlanner, num_complex::Complex};
use std::num::NonZero;

/// Converts HSL (h: 0-360, s: 0-1, l: 0-1) to RGB (0-255 each)
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r1, g1, b1) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

/// Maps a frequency (Hz) to a hue (0-360 degrees).
/// Uses a log scale since human pitch perception is logarithmic, not linear.
fn freq_to_hue(freq: f32) -> f32 {
    let min_freq = 20.0_f32;
    let max_freq = 8000.0_f32; // most musically relevant content lives under here
    let clamped = freq.clamp(min_freq, max_freq);
    let log_pos = (clamped.ln() - min_freq.ln()) / (max_freq.ln() - min_freq.ln());
    log_pos * 360.0
}

/// Finds the dominant frequency in a chunk of samples via FFT.
fn dominant_frequency(chunk: &[f32], sample_rate: NonZero<u32>) -> f32 {
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(chunk.len());

    let mut buffer: Vec<Complex<f32>> = chunk
        .iter()
        .map(|&s| Complex { re: s, im: 0.0 })
        .collect();

    fft.process(&mut buffer);

    let (bin, _magnitude) = buffer
        .iter()
        .take(buffer.len() / 2) // FFT output mirrors after the midpoint
        .enumerate()
        .skip(1) // skip bin 0 (DC offset / zero-Hz component, not useful)
        .max_by(|(_, a), (_, b)| a.norm().partial_cmp(&b.norm()).unwrap())
        .unwrap_or((0, &Complex { re: 0.0, im: 0.0 }));

    bin as f32 * sample_rate.get() as f32 / chunk.len() as f32
}

/// Generates a color palette from raw audio samples.
/// Splits the recording into `num_colors` equal chunks, and derives one
/// color per chunk from its dominant frequency (hue) and loudness (lightness).
pub fn generate_palette(samples: &[f32], sample_rate: NonZero<u32>, num_colors: usize) -> Vec<(u8, u8, u8)> {
    let chunk_size = samples.len() / num_colors;
    if chunk_size == 0 {
        return Vec::new();
    }

    samples
        .chunks(chunk_size)
        .take(num_colors)
        .map(|chunk| {
            let freq = dominant_frequency(chunk, sample_rate);
            let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();

            let hue = freq_to_hue(freq);
            let lightness = rms.clamp(0.05, 0.85); // avoid pure black/white extremes
            let saturation = 0.75;

            hsl_to_rgb(hue, saturation, lightness)
        })
        .collect()
}
