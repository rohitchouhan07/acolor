pub mod inputs;
pub mod properties;

use std::{error::Error, num::NonZero, sync::mpsc, thread, time::Duration};
use crate::colors;
use rodio::{Source, microphone::MicrophoneBuilder};
use indicatif::{ProgressBar, ProgressStyle};

use crate::{RunOpts};
pub fn check_inputs_mode() -> Result<(), Box<dyn Error>> {
    inputs::print_input_devices()?;
    Ok(())
}

/// Averages interleaved channels into a single mono signal.
/// e.g. stereo [L0, R0, L1, R1, ...] -> [(L0+R0)/2, (L1+R1)/2, ...]
pub(crate) fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

struct Recording {
    samples: Vec<f32>, // interleaved, `channels` samples per frame
    channels: NonZero<u16>,
    sample_rate: NonZero<u32>,
}

/// Extra time we wait past the requested duration before giving up on the mic.
const RECORD_GRACE: Duration = Duration::from_secs(5);

/// Records `secs` seconds from the default microphone.
///
/// rodio's microphone has two failure modes that `take_duration` hides:
/// - If the mic never delivers audio (device busy, suspended, muted at the
///   driver level) its iterator sleeps forever, so the program hang
/// - If the stream hits an error (xrun, device unplugged) it prints
///   "audio stream error: ..." and ends the iterator early, so you silently
///   get a recording shorter than requested.
///
/// To guard against both, the mic lives on its own thread and we wa
/// a timeout, then check that we actually got (nearly) all the audio.
fn record(secs: u8) -> Result<Recording, Box<dyn Error>> {
    let (tx, rx) = mpsc::channel::<Result<Recording, String>>();

    // The mic is opened inside the thread because cpal streams can't be
    // moved between threads on every platform.
    thread::spawn(move || {
        let result = (|| -> Result<Recording, Box<dyn Error>> {
            let mic = MicrophoneBuilder::new()
                .default_device()?
                .default_config()?
                .open_stream()?;
            let channels = mic.channels();
            let sample_rate = mic.sample_rate();
            let samples = mic.take_duration(Duration::from_secs(secs as u64)).collect();
            Ok(Recording { samples, channels, sample_rate })
        })();
        // Box<dyn Error> isn't Send, so pass errors back as text.
        // If the receiver already gave up, there's nobody to tell,
        let _ = tx.send(result.map_err(|e| e.to_string()));
    });

    let recording = match rx.recv_timeout(Duration::from_secs(secs as u64) + RECORD_GRACE) {
        Ok(Ok(rec)) => rec,
        Ok(Err(e)) => return Err(format!("could not record from microphone: {e}").into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err("microphone stopped delivering audio (timed out). \
                        Check that no other app has exclusive access to it, \
                        or pick another device with `-m check-inputs`"
                .into());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("recording thread crashed".into());
        }
    };

    check_length(
        recording.samples.len(),
        recording.channels.get(),
        recording.sample_rate.get(),
        secs,
    )?;
    Ok(recording)
}

/// Fails if the recording is noticeably shorter than requested, which means
/// the audio stream errored partway through.
fn check_length(len: usize, channels: u16, sample_rate: u32,secs: u8) -> Result<(), String> {    
    let expected = secs as usize * sample_rate as usize * channels as usize;
    // Allow 5% slack for startup latency and rounding in take_duration
    if len * 100 < expected * 95 {
        let got_secs = len as f32 / (sample_rate as f32 * channels as f32);
        return Err(format!(
            "recording stopped early: got {got_secs:.1}s of {secs}s. \
             The audio stream reported an error (see the message above)"
        ));
    }
    Ok(())
}

fn trim_start(samples: &[f32], sample_rate: NonZero<u32>, channels: u16, ms: u32) -> &[f32] {
    let skip = (sample_rate.get() as u64 * ms as u64 / 1000) as usize * channels as usize;

    if skip < samples.len() {
        &samples[skip..]
    } else {
        samples // recording shorter than the trim window, so leave
    }
}

pub fn main_mode(opts: RunOpts) -> Result<(), Box<dyn Error>> {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );

    pb.set_message("Sampling audio...");
    pb.enable_steady_tick(Duration::from_millis(80));

    let recording = record(opts.sample_time);
    pb.finish_and_clear();
    let Recording { samples, channels, sample_rate } = recording?;
    println!("✔ Sampling complete!");

    if opts.debug {
        println!("Run options: {:?}", opts);
        println!("channels: {channels}, sample rate: {sample_rate} Hz");
        println!("recorded {} samples", samples.len());
    }

    let mono = to_mono(&samples, channels.get());
    let trimmed = trim_start(&mono, sample_rate, 1, 500);
    let palette = colors::generate_palette(trimmed, sample_rate, &opts)?;
    println!("Generated palette:");
    
    for (r, g, b) in &palette {
        println!("  rgb({}, {}, {})", r, g, b);
    }
    colors::save_palette_card(&palette, "palette.png")?;
    println!("Saved palette.png");
  
    if opts.debug == true {
        properties::print_amplitude_loudness(&samples);
        println!("Playing recording...");
        let handle = rodio::DeviceSinkBuilder::open_default_sink()
                .expect("open default audio stream");
        let player = rodio::Player::connect_new(&handle.mixer());
        let source = rodio::buffer::SamplesBuffer::new(channels, sample_rate, samples);
        player.append(source);
        player.sleep_until_end();
    }
    Ok(())
}

