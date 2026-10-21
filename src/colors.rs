use rustfft::{FftPlanner, num_complex::Complex};
use std::f32::consts::PI;
use std::num::NonZero;

use crate::RunOpts;

// hsl_to_rgb and freq_to_hue unchanged...

struct Band {
    name: &'static str,
    low_hz: f32,
    high_hz: f32,
}

const BANDS: [Band; 3] = [
    Band { name: "low",  low_hz: 20.0,   high_hz: 250.0 },
    Band { name: "mid",  low_hz: 250.0,  high_hz: 2000.0 },
    Band { name: "high", low_hz: 2000.0, high_hz: 8000.0 },
];

struct BandStats {
    peak_freq: f32,  // strongest frequency inside the band
    avg_power: f32,  // mean power per bin, so wide bands aren't favored
}

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

/// One FFT over the whole recording. Returns the magnitude of each bin
/// up to Nyquist (the second half of the FFT output just mirrors the first).
fn magnitude_spectrum(samples: &[f32]) -> Vec<f32> {
    let n = samples.len();
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n);

    let mut buffer: Vec<Complex<f32>> = samples
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            // Hann window: reduces spectral leakage so loud low frequencies
            // don't smear into the mid/high bands
            let w = 0.5 * (1.0 - (2.0 * PI * i as f32 / (n as f32 - 1.0)).cos());
            Complex { re: s * w, im: 0.0 }
        })
        .collect();

    fft.process(&mut buffer);

    buffer.iter().take(n / 2).map(|c| c.norm()).collect()
}

fn analyze_band(
    spectrum: &[f32],
    n: usize,
    sample_rate: NonZero<u32>,
    band: &Band,
) -> Option<BandStats> {
    let hz_per_bin = sample_rate.get() as f32 / n as f32;

    // Convert the band's Hz range into bin indices (skip bin 0, the DC offset)
    let start = ((band.low_hz / hz_per_bin).ceil() as usize).max(1);
    let end = ((band.high_hz / hz_per_bin).floor() as usize).min(spectrum.len());
    if start >= end {
        return None;
    }

    let slice = &spectrum[start..end];

    let (peak_offset, _) = slice
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))?;

    let avg_power = slice.iter().map(|m| m * m).sum::<f32>() / slice.len() as f32;

    Some(BandStats {
        peak_freq: (start + peak_offset) as f32 * hz_per_bin,
        avg_power,
    })
}

/// Generates up to three colors (low, mid, high) from the recording's spectrum.
/// Hue comes from each band's strongest frequency, lightness from how loud
/// the band is relative to the others.
pub fn generate_palette(
    samples: &[f32],
    sample_rate: NonZero<u32>,
    opts: &RunOpts,
) -> Vec<(u8, u8, u8)> {
    if samples.len() < 2 {
        return Vec::new();
    }

    let spectrum = magnitude_spectrum(samples);

    let stats: Vec<Option<BandStats>> = BANDS
        .iter()
        .map(|b| analyze_band(&spectrum, samples.len(), sample_rate, b))
        .collect();

    if opts.debug {
        for (band, s) in BANDS.iter().zip(&stats) {
            match s {
                Some(s) => println!(
                    "{:>4}: peak {:.0} Hz, avg power {:.4}",
                    band.name, s.peak_freq, s.avg_power
                ),
                None => println!("{:>4}: no bins in range", band.name),
            }
        }
    }

    let max_power = stats
        .iter()
        .flatten()
        .map(|s| s.avg_power)
        .fold(0.0_f32, f32::max);

    stats
        .into_iter()
        .flatten()
        .map(|s| {
            let ratio = if max_power > 0.0 { s.avg_power / max_power } else { 0.0 };

            let hue = freq_to_hue(s.peak_freq);
            // powf(0.25) compresses the range so quiet bands stay visible
            let lightness = 0.2 + 0.6 * ratio.powf(0.25);
            let saturation = 0.75;

            hsl_to_rgb(hue, saturation, lightness)
        })
        .collect()
}
