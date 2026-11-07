use rustfft::{FftPlanner, num_complex::Complex};
use std::f32::consts::PI;
use std::num::NonZero;
use image::{Rgba, RgbaImage};
use std::error::Error;

use crate::RunOpts;

type Rgb = (u8, u8, u8);

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
    pitch_class: usize, // dominant note in the band (0 = C ... 11 = B)
    avg_power: f32,  // mean power per bin, so wide bands aren't favored
    flatness: f32,   // 0 = pure tone, ~0.56 = white noise (see spectral_flatness)
}

/// Spectral flatness of a band: geometric mean / arithmetic mean of power.
/// A pure tone puts all its power in a few bins -> close to 0.
/// Noise spreads it evenly -> high. For a single FFT of white noise the
/// expected value is about e^-0.5772 = 0.56, so that's our "fully noisy" point.
fn spectral_flatness(power: &[f32]) -> f32 {
    const EPS: f32 = 1e-12; // keeps ln() away from zero for silent bins
    let n = power.len() as f32;
    let arith = power.iter().sum::<f32>() / n;
    if arith <= EPS {
        return 1.0; // silence: treat as featureless noise
    }
    let geo = (power.iter().map(|p| (p + EPS).ln()).sum::<f32>() / n).exp();
    geo / arith
}

const NOISE_FLATNESS: f32 = 0.56;
/// Below this loudness level (0.1 = -55 dBFS) we refuse to make a palette.
/// Typical mic self-noise in a quiet room sits around -60 to -70 dBFS.
const MIN_LEVEL: f32 = 0.1;

/// Maps flatness to saturation: tonal bands are vivid, noisy bands go grey.
fn saturation_from_flatness(flatness: f32) -> f32 {
    let tonality = 1.0 - (flatness / NOISE_FLATNESS).clamp(0.0, 1.0);
    0.15 + 0.8 * tonality
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Maps RMS to a 0-1 loudness level on a dB scale.
/// -60 dBFS (near silence) -> 0.0, -10 dBFS (very loud) -> 1.0.
fn level_from_rms(rms: f32) -> f32 {
    const FLOOR_DB: f32 = -60.0;
    const CEIL_DB: f32 = -10.0;
    let db = 20.0 * rms.max(1e-9).log10();
    ((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0)
}

fn lerp(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

/// 0.0 outside the rounded corners, 1.0 inside, with a 1px antialiased edge.
fn corner_alpha(x: u32, y: u32, w: u32, h: u32, r: f32) -> f32 {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let cx = if px < r { r } else if px > w as f32 - r { w as f32 - r } else { return 1.0 };
    let cy = if py < r { r } else if py > h as f32 - r { h as f32 - r } else { return 1.0 };
    let dist = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    (r - dist + 0.5).clamp(0.0, 1.0)
}

pub fn save_palette_card(palette: &[(u8, u8, u8)], path: &str) -> Result<(), Box<dyn Error>> {
    const W: u32 = 1200;
    const H: u32 = 630;
    const SWATCH_H: u32 = 140; // height of the solid swatch strip at the bottom
    const RADIUS: f32 = 32.0;

    let n = palette.len();
    if n == 0 {
        return Err("palette is empty".into());
    }

    let img = RgbaImage::from_fn(W, H, |x, y| {
        let (r, g, b) = if y >= H - SWATCH_H {
            // Bottom strip: one solid block per color
            palette[(x as usize * n / W as usize).min(n - 1)]
        } else if n == 1 {
            palette[0]
        } else {
            // Gradient: find which two colors this x sits between
            let t = x as f32 / (W - 1) as f32 * (n - 1) as f32;
            let i = (t.floor() as usize).min(n - 2);
            let f = t - i as f32;
            let f = f * f * (3.0 - 2.0 * f); // smoothstep for softer transitions
            let (c0, c1) = (palette[i], palette[i + 1]);
            (lerp(c0.0, c1.0, f), lerp(c0.1, c1.1, f), lerp(c0.2, c1.2, f))
        };

        let alpha = (corner_alpha(x, y, W, H, RADIUS) * 255.0) as u8;
        Rgba([r, g, b, alpha])
    });

    img.save(path)?;
    Ok(())
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

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Maps a frequency (Hz) to its pitch class: 0 = C, 1 = C#, ... 11 = B.
/// Octave doesn't matter, so 110 Hz, 220 Hz and 440 Hz are all A (9).
fn freq_to_pitch_class(freq: f32) -> usize {
    // MIDI note number: A4 = 440 Hz = 69, one step per semitone
    let midi = 69.0 + 12.0 * (freq / 440.0).log2();
    (midi.round() as i32).rem_euclid(12) as usize
}

/// Maps a pitch class to a hue by walking the circle of fifths
/// (C, G, D, A, E, ...). Notes that sound good together sit next to
/// each other on the circle, so harmonious audio gives harmonious colors.
fn pitch_class_to_hue(pc: usize) -> f32 {
    let fifths_pos = (pc * 7) % 12; // 7 semitones = a perfect fifth
    fifths_pos as f32 * 30.0
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

    let power: Vec<f32> = slice.iter().map(|m| m * m).collect();
    let avg_power = power.iter().sum::<f32>() / power.len() as f32;
    let flatness = spectral_flatness(&power);

    // Chroma: fold every bin's power into one of 12 pitch classes, then pick
    // the strongest. More robust than the single peak bin, since a note's
    // energy is spread over neighbouring bins and its octaves.
    let mut chroma = [0.0_f32; 12];
    for (i, m) in slice.iter().enumerate() {
        let freq = (start + i) as f32 * hz_per_bin;
        chroma[freq_to_pitch_class(freq)] += m * m;
    }
    let (pitch_class, _) = chroma
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))?;

    Some(BandStats {
        peak_freq: (start + peak_offset) as f32 * hz_per_bin,
        pitch_class,
        avg_power,
        flatness,
    })
}

/// Hue comes from each band's dominant note (via the circle of fifths),
/// saturation from how tonal vs noisy the band is, and lightness from the
/// overall loudness combined with how loud the band is relative to the others.
///
/// Returns an error instead of a palette when the recording is too short or
/// too quiet to say anything meaningful (otherwise you'd get colors made
/// from mic hiss).
pub fn generate_palette(
    samples: &[f32],
    sample_rate: NonZero<u32>,
    opts: &RunOpts,
) -> Result<Vec<Rgb>, Box<dyn Error>> {
    // Need at least 0.1s of audio for the low band to have any FFT bins
    if samples.len() < (sample_rate.get() / 10) as usize {
        return Err("recording is too short to analyze".into());
    }

    let level = level_from_rms(rms(samples));
    if level < MIN_LEVEL {
        return Err(format!(
            "recording is too quiet (level {level:.2}, need at least {MIN_LEVEL:.2}). \
             Turn the volume up, move closer to the mic, or check the input \
             device with `-m check-inputs`"
        )
        .into());
    }

    let spectrum = magnitude_spectrum(samples);

    let stats: Vec<Option<BandStats>> = BANDS
        .iter()
        .map(|b| analyze_band(&spectrum, samples.len(), sample_rate, b))
        .collect();

    let level = level_from_rms(rms(samples));

    if opts.debug {
        println!("level: {:.2} (rms {:.4})", level, rms(samples));
        for (band, s) in BANDS.iter().zip(&stats) {
            match s {
                Some(s) => println!(
                    "{:>4}: peak {:.0} Hz, note {}, avg power {:.4}, flatness {:.3}",
                    band.name, s.peak_freq, NOTE_NAMES[s.pitch_class], s.avg_power, s.flatness
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

    let palette = stats
        .into_iter()
        .flatten()
        .map(|s| {
            let ratio = if max_power > 0.0 { s.avg_power / max_power } else { 0.0 };

            let hue = pitch_class_to_hue(s.pitch_class);
            // powf(0.25) compresses the range so quiet bands stay visible
            // Overall loudness sets how bright the palette can get; each band's
            // share of the power scales it down from there. powf(0.25)
            // compresses the range so quiet bands stay visible.
            // Capped at 0.75 so the loudest band doesn't wash out to white.
            let lightness = 0.15 + 0.6 * level * ratio.powf(0.25);
            let saturation = saturation_from_flatness(s.flatness);

            hsl_to_rgb(hue, saturation, lightness)
        })
        .collect();

    Ok(palette)
}
