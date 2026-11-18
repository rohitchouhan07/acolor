use std::{error::Error, f32::consts::PI, num::NonZero, ops::Range};

use rustfft::{FftPlanner, num_complex::Complex};

pub type Rgb = (u8, u8, u8);

// ---------------------------------------------------------------------------
// Frequency bands
// ---------------------------------------------------------------------------

/// Range of frequencies we turn into colors. Below 30 Hz is mostly rumble,
/// above 8 kHz is mostly cymbal/hiss with little pitch information.
const MIN_HZ: f32 = 30.0;
const MAX_HZ: f32 = 8000.0;

/// Splits MIN_HZ..MAX_HZ into `n` bands of equal width on a log scale, so
/// each band covers the same number of octaves (the way we hear pitch).
fn bands(n: usize) -> Vec<(f32, f32)> {
    let ratio = (MAX_HZ / MIN_HZ).powf(1.0 / n as f32);
    (0..n)
        .map(|i| (MIN_HZ * ratio.powi(i as i32), MIN_HZ * ratio.powi(i as i32 + 1)))
        .collect()
}

// ---------------------------------------------------------------------------
// Spectrum analysis
// ---------------------------------------------------------------------------

/// FFT window length. At 44.1 kHz this is ~1.5s per window and 0.67 Hz per
/// bin, fine enough to tell neighbouring notes apart even in the bass.
const FFT_SIZE: usize = 65_536;

struct BandStats {
    peak_freq: f32,     // strongest frequency inside the band
    chroma: [f32; 12],  // power per pitch class (0 = C ... 11 = B)
    power: f32,         // total power in the band
    flatness: f32,      // 0 = pure tone, ~0.56 = white noise (see spectral_flatness)
}

/// Converts a Hz range into FFT bin indices (skipping bin 0, the DC offset).
fn bin_range(low_hz: f32, high_hz: f32, hz_per_bin: f32, bins: usize) -> Option<Range<usize>> {
    let start = ((low_hz / hz_per_bin).ceil() as usize).max(1);
    let end = ((high_hz / hz_per_bin).floor() as usize).min(bins);
    (start < end).then_some(start..end)
}

/// Analyzes each band using Welch's method: the audio is cut into
/// overlapping windows, each window gets its own FFT, and the power spectra
/// are averaged. Compared to one giant FFT this is much faster on long files,
/// and the result reflects the whole track instead of being dominated by
/// whichever single moment happened to be loudest.
fn analyze_bands(
    samples: &[f32],
    sample_rate: NonZero<u32>,
    bands: &[(f32, f32)],
) -> Vec<Option<BandStats>> {
    let size = FFT_SIZE.min(samples.len());
    let hop = (size / 2).max(1); // 50% overlap
    let bins = size / 2; // bins above Nyquist mirror the ones below
    let hz_per_bin = sample_rate.get() as f32 / size as f32;

    let ranges: Vec<Option<Range<usize>>> = bands
        .iter()
        .map(|&(lo, hi)| bin_range(lo, hi, hz_per_bin, bins))
        .collect();

    // Hann window: reduces spectral leakage so loud low frequencies
    // don't smear into the bands above them
    let window: Vec<f32> = (0..size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (size as f32 - 1.0).max(1.0)).cos()))
        .collect();

    let fft = FftPlanner::new().plan_fft_forward(size);
    let mut buffer = vec![Complex::new(0.0, 0.0); size];
    let mut avg_power = vec![0.0_f32; bins];
    // Flatness is measured per window (that's what the 0.56 noise reference
    // assumes) and averaged, weighted by power so silent gaps don't count.
    let mut flatness_sum = vec![0.0_f32; bands.len()];
    let mut flatness_weight = vec![0.0_f32; bands.len()];
    let mut windows = 0;

    for start in (0..=samples.len() - size).step_by(hop) {
        for ((b, &s), &w) in buffer.iter_mut().zip(&samples[start..start + size]).zip(&window) {
            *b = Complex::new(s * w, 0.0);
        }
        fft.process(&mut buffer);

        let power: Vec<f32> = buffer[..bins].iter().map(|c| c.norm_sqr()).collect();
        for (avg, p) in avg_power.iter_mut().zip(&power) {
            *avg += p;
        }
        for (i, range) in ranges.iter().enumerate() {
            if let Some(range) = range {
                let slice = &power[range.clone()];
                let weight = slice.iter().sum::<f32>();
                flatness_sum[i] += spectral_flatness(slice) * weight;
                flatness_weight[i] += weight;
            }
        }
        windows += 1;
    }
    for p in &mut avg_power {
        *p /= windows as f32;
    }

    ranges
        .iter()
        .enumerate()
        .map(|(i, range)| {
            let range = range.clone()?;
            let slice = &avg_power[range.clone()];

            let (peak_offset, _) = slice
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.total_cmp(b))?;

            // Chroma: fold every bin's power into one of 12 pitch classes.
            // More robust than the single peak bin, since a note's energy
            // is spread over neighbouring bins and its octaves.
            let mut chroma = [0.0_f32; 12];
            for (offset, p) in slice.iter().enumerate() {
                let freq = (range.start + offset) as f32 * hz_per_bin;
                chroma[freq_to_pitch_class(freq)] += p;
            }

            let flatness = if flatness_weight[i] > 0.0 {
                flatness_sum[i] / flatness_weight[i]
            } else {
                1.0
            };

            Some(BandStats {
                peak_freq: (range.start + peak_offset) as f32 * hz_per_bin,
                chroma,
                // Total (not per-bin) power: log-spaced bands each span the
                // same number of octaves, so totals compare fairly
                power: slice.iter().sum(),
                flatness,
            })
        })
        .collect()
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

// ---------------------------------------------------------------------------
// Sound -> color mapping
// ---------------------------------------------------------------------------

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

/// A band only switches to a less dominant note if that note has at least
/// this fraction of the band's strongest note's power. Stops us inventing
/// colors from notes that are barely there (e.g. a single sine wave).
const DISTINCT_NOTE_MIN: f32 = 0.25;

/// Picks the strongest note in each band, preferring notes that earlier
/// bands haven't used yet. Without this, a song in one key tends to give
/// every band the same root note, and so the same hue.
fn pick_notes(chromas: &[[f32; 12]]) -> Vec<usize> {
    let mut used = [false; 12];
    chromas
        .iter()
        .map(|chroma| {
            let mut ranked: Vec<usize> = (0..12).collect();
            ranked.sort_by(|&a, &b| chroma[b].total_cmp(&chroma[a]));
            let top = ranked[0];
            let pick = ranked
                .iter()
                .copied()
                .take_while(|&pc| chroma[pc] >= chroma[top] * DISTINCT_NOTE_MIN)
                .find(|&pc| !used[pc])
                .unwrap_or(top);
            used[pick] = true;
            pick
        })
        .collect()
}

const NOISE_FLATNESS: f32 = 0.56;

/// Maps flatness to saturation: tonal bands are vivid, noisy bands go grey.
fn saturation_from_flatness(flatness: f32) -> f32 {
    let tonality = 1.0 - (flatness / NOISE_FLATNESS).clamp(0.0, 1.0);
    0.15 + 0.8 * tonality
}

/// Below this loudness level (0.1 = -55 dBFS) we refuse to make a palette.
/// Typical mic self-noise in a quiet room sits around -60 to -70 dBFS.
const MIN_LEVEL: f32 = 0.1;

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

/// Converts HSL (h: 0-360, s: 0-1, l: 0-1) to RGB (0-255 each)
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Rgb {
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

/// Generates a palette of up to `n_colors` colors, one per frequency band,
/// ordered from bass to treble.
///
/// Hue comes from each band's dominant note (via the circle of fifths),
/// saturation from how tonal vs noisy the band is, and lightness from the
/// overall loudness combined with how loud the band is relative to the others.
///
/// Returns an error instead of a palette when the audio is too short or
/// too quiet to say anything meaningful (otherwise you'd get colors made
/// from mic hiss).
pub fn generate_palette(
    samples: &[f32],
    sample_rate: NonZero<u32>,
    n_colors: usize,
    debug: bool,
) -> Result<Vec<Rgb>, Box<dyn Error>> {
    // Need at least 0.1s of audio for the bass bands to have any FFT bins
    if samples.len() < (sample_rate.get() / 10) as usize {
        return Err("audio is too short to analyze".into());
    }

    let level = level_from_rms(rms(samples));
    if level < MIN_LEVEL {
        return Err(format!(
            "audio is too quiet (level {level:.2}, need at least {MIN_LEVEL:.2}). \
             Turn the volume up, move closer to the mic, or pick another \
             input with --device"
        )
        .into());
    }

    let bands = bands(n_colors);
    // Bands too narrow for the FFT resolution (very short audio) are dropped
    let (bands, stats): (Vec<&(f32, f32)>, Vec<BandStats>) = bands
        .iter()
        .zip(analyze_bands(samples, sample_rate, &bands))
        .filter_map(|(band, s)| Some((band, s?)))
        .unzip();

    let notes = pick_notes(&stats.iter().map(|s| s.chroma).collect::<Vec<_>>());
    let max_power = stats.iter().map(|s| s.power).fold(0.0_f32, f32::max);

    if debug {
        eprintln!(
            "level: {level:.2} (rms {:.4}), {:.1}s at {} Hz",
            rms(samples),
            samples.len() as f32 / sample_rate.get() as f32,
            sample_rate
        );
        for ((band, s), &note) in bands.iter().zip(&stats).zip(&notes) {
            eprintln!(
                "{:>5.0}-{:<5.0} Hz: peak {:>5.0} Hz, note {:<2}, power {:.2e}, flatness {:.3}",
                band.0, band.1, s.peak_freq, NOTE_NAMES[note], s.power, s.flatness
            );
        }
    }

    let palette = stats
        .iter()
        .zip(&notes)
        .map(|(s, &note)| {
            let ratio = if max_power > 0.0 { s.power / max_power } else { 0.0 };

            let hue = pitch_class_to_hue(note);
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

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn sr() -> NonZero<u32> {
        NonZero::new(SR).unwrap()
    }

    fn sine(freq: f32, secs: f32) -> Vec<f32> {
        let n = (SR as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / SR as f32).sin())
            .collect()
    }

    /// Deterministic white noise in [-1, 1] (xorshift, no rand crate needed)
    fn noise(secs: f32) -> Vec<f32> {
        let mut x: u32 = 0x1234_5678;
        (0..(SR as f32 * secs) as usize)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as f32 / u32::MAX as f32 * 2.0 - 1.0
            })
            .collect()
    }

    fn scaled(samples: Vec<f32>, amp: f32) -> Vec<f32> {
        samples.into_iter().map(|s| s * amp).collect()
    }

    /// Stats for a single 250-2000 Hz band
    fn mid_band(samples: &[f32]) -> BandStats {
        analyze_bands(samples, sr(), &[(250.0, 2000.0)])
            .pop()
            .flatten()
            .expect("band should have bins")
    }

    fn top_note(chroma: &[f32; 12]) -> usize {
        (0..12).max_by(|&a, &b| chroma[a].total_cmp(&chroma[b])).unwrap()
    }

    // --- bands & spectrum ---

    #[test]
    fn bands_cover_the_range_contiguously() {
        let b = bands(5);
        assert_eq!(b.len(), 5);
        assert!((b[0].0 - MIN_HZ).abs() < 1e-3);
        assert!((b[4].1 - MAX_HZ).abs() < 1e-1);
        for pair in b.windows(2) {
            assert!((pair[0].1 - pair[1].0).abs() < 1e-3);
        }
    }

    #[test]
    fn sine_440hz_peaks_at_440_and_is_note_a() {
        let s = mid_band(&sine(440.0, 1.0));
        assert!((s.peak_freq - 440.0).abs() < 2.0, "peak was {} Hz", s.peak_freq);
        assert_eq!(NOTE_NAMES[top_note(&s.chroma)], "A");
    }

    #[test]
    fn long_audio_uses_multiple_windows() {
        // 10s at 48kHz = 480k samples, several 64k windows. Still finds A.
        let s = mid_band(&sine(440.0, 10.0));
        assert!((s.peak_freq - 440.0).abs() < 1.0, "peak was {} Hz", s.peak_freq);
    }

    #[test]
    fn stereo_without_downmix_halves_frequencies() {
        // Documents the v0.1 bug: interleaved stereo treated as mono
        let interleaved: Vec<f32> = sine(440.0, 1.0).into_iter().flat_map(|s| [s, s]).collect();
        let s = analyze_bands(&interleaved, sr(), &[(20.0, 250.0)]).pop().flatten().unwrap();
        assert!((s.peak_freq - 220.0).abs() < 2.0, "peak was {} Hz", s.peak_freq);
    }

    #[test]
    fn sine_is_tonal_noise_is_flat() {
        let tone = mid_band(&sine(440.0, 3.0)).flatness;
        let hiss = mid_band(&noise(3.0)).flatness;
        assert!(tone < 0.05, "sine flatness was {tone}");
        assert!(hiss > 0.4, "noise flatness was {hiss}");
        assert!(saturation_from_flatness(tone) > 0.85);
        assert!(saturation_from_flatness(hiss) < 0.4);
    }

    // --- notes & hues ---

    #[test]
    fn pitch_classes() {
        assert_eq!(freq_to_pitch_class(440.0), 9);   // A4
        assert_eq!(freq_to_pitch_class(110.0), 9);   // A2, octave ignored
        assert_eq!(freq_to_pitch_class(261.63), 0);  // C4
        assert_eq!(freq_to_pitch_class(392.0), 7);   // G4
        assert_eq!(freq_to_pitch_class(27.5), 9);    // A0, low edge of piano
    }

    #[test]
    fn circle_of_fifths_hues() {
        assert_eq!(pitch_class_to_hue(0), 0.0);   // C
        assert_eq!(pitch_class_to_hue(7), 30.0);  // G, one fifth up
        assert_eq!(pitch_class_to_hue(2), 60.0);  // D, two fifths up
        assert_eq!(pitch_class_to_hue(5), 330.0); // F, one fifth down
    }

    #[test]
    fn bands_prefer_distinct_notes() {
        // A (9) strongest, E (4) also clearly present
        let mut chroma = [0.0; 12];
        chroma[9] = 1.0;
        chroma[4] = 0.6;
        assert_eq!(pick_notes(&[chroma, chroma]), vec![9, 4]);
    }

    #[test]
    fn faint_notes_are_not_promoted() {
        // E is too weak to count, so the second band repeats A
        let mut chroma = [0.0; 12];
        chroma[9] = 1.0;
        chroma[4] = 0.1;
        assert_eq!(pick_notes(&[chroma, chroma]), vec![9, 9]);
    }

    // --- loudness ---

    #[test]
    fn loudness_levels() {
        assert_eq!(level_from_rms(0.0), 0.0);    // silence
        assert_eq!(level_from_rms(0.001), 0.0);  // -60 dBFS
        assert_eq!(level_from_rms(1.0), 1.0);    // full scale
        let mid = level_from_rms(0.01);          // -40 dBFS
        assert!((mid - 0.4).abs() < 1e-4, "got {mid}");
    }

    // --- whole palette ---

    #[test]
    fn palette_has_requested_size() {
        let audio = scaled(noise(3.0), 0.3);
        for n in [1, 3, 5, 8, 12] {
            assert_eq!(generate_palette(&audio, sr(), n, false).unwrap().len(), n);
        }
    }

    #[test]
    fn same_input_gives_same_palette() {
        let audio = scaled(sine(440.0, 2.0), 0.3);
        let a = generate_palette(&audio, sr(), 5, false).unwrap();
        let b = generate_palette(&audio, sr(), 5, false).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn louder_audio_gives_lighter_palette() {
        let brightness = |amp: f32| -> u32 {
            generate_palette(&scaled(sine(440.0, 1.0), amp), sr(), 5, false)
                .unwrap()
                .iter()
                .map(|&(r, g, b)| r as u32 + g as u32 + b as u32)
                .sum()
        };
        assert!(brightness(0.5) > brightness(0.01));
    }

    #[test]
    fn silence_is_rejected() {
        let err = generate_palette(&vec![0.0; SR as usize], sr(), 5, false).unwrap_err();
        assert!(err.to_string().contains("too quiet"), "got: {err}");
    }

    #[test]
    fn faint_hiss_is_rejected() {
        // Noise at roughly -70 dBFS, like a mic in a silent room
        assert!(generate_palette(&scaled(noise(1.0), 0.0005), sr(), 5, false).is_err());
    }

    #[test]
    fn quiet_but_real_sound_is_accepted() {
        // -43 dBFS sine: quiet, but well above the noise floor
        assert!(generate_palette(&scaled(sine(440.0, 1.0), 0.01), sr(), 5, false).is_ok());
    }

    #[test]
    fn too_short_is_rejected() {
        let err = generate_palette(&sine(440.0, 0.01), sr(), 5, false).unwrap_err();
        assert!(err.to_string().contains("too short"), "got: {err}");
    }
}
