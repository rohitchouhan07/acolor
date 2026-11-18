use std::{
    error::Error,
    fs::File,
    io::{Cursor, Read},
    num::NonZero,
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use indicatif::{ProgressBar, ProgressStyle};
use rodio::{
    Decoder, Source,
    microphone::{Input, MicrophoneBuilder, available_inputs},
};

/// Mono audio ready for analysis.
pub struct Audio {
    pub mono: Vec<f32>,
    pub sample_rate: NonZero<u32>,
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

/// Decodes an audio file (or stdin when `path` is "-") into mono samples.
pub fn load(path: &Path) -> Result<Audio, Box<dyn Error>> {
    if path == Path::new("-") {
        // Decoders need to seek, and stdin can't, so buffer it all first
        let mut bytes = Vec::new();
        std::io::stdin().read_to_end(&mut bytes)?;
        if bytes.is_empty() {
            return Err("no audio data on stdin".into());
        }
        let decoder = Decoder::try_from(Cursor::new(bytes))
            .map_err(|e| format!("could not decode stdin: {e}"))?;
        return Ok(from_source(decoder));
    }

    let file =
        File::open(path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    let decoder =
        Decoder::try_from(file).map_err(|e| format!("could not decode {}: {e}", path.display()))?;
    Ok(from_source(decoder))
}

fn from_source<S: Source>(source: S) -> Audio {
    let channels = source.channels();
    let sample_rate = source.sample_rate();
    let samples: Vec<f32> = source.collect();
    Audio {
        mono: to_mono(&samples, channels.get()),
        sample_rate,
    }
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

// ---------------------------------------------------------------------------
// Microphone
// ---------------------------------------------------------------------------

pub fn print_devices() -> Result<(), Box<dyn Error>> {
    let inputs = available_inputs()?;
    if inputs.is_empty() {
        println!("No input devices found");
    }
    for (i, input) in inputs.iter().enumerate() {
        println!("{i}: {input}");
    }
    Ok(())
}

/// Finds an input device by index ("2") or case-insensitive name fragment ("usb").
/// Indices match the list printed by `acolor devices`.
fn find_device(query: &str) -> Result<Input, Box<dyn Error>> {
    let inputs = available_inputs()?;

    if let Ok(index) = query.parse::<usize>() {
        return inputs
            .into_iter()
            .nth(index)
            .ok_or_else(|| format!("no input device #{index} (see `acolor devices`)").into());
    }

    let needle = query.to_lowercase();
    inputs
        .into_iter()
        .find(|input| input.to_string().to_lowercase().contains(&needle))
        .ok_or_else(|| format!("no input device matching \"{query}\" (see `acolor devices`)").into())
}

/// Extra time we wait past the requested duration before deciding the mic
/// thread is stuck (e.g. blocked forever waiting for a device that never
/// starts).
const RECORD_GRACE: Duration = Duration::from_secs(5);

/// If no audio arrives for this long, the stream is dead, not just hiccuping.
const STALL_TIMEOUT: Duration = Duration::from_secs(1);

/// Skip the first half second: mics often click or ramp up when opened.
const TRIM_START_MS: u32 = 500;

/// Records `secs` seconds from the given (or default) microphone.
///
/// rodio's microphone has two quirks we work around:
/// - After *any* stream error it ends the iterator as soon as its buffer is
///   momentarily empty, even for errors the stream recovers from. On ALSA,
///   "`alsa::poll()` spuriously returned" is one of those: harmless, but it
///   used to cut recordings off at 0s. So we keep reading past a `None`,
///   and only give up once no audio has arrived for STALL_TIMEOUT.
///   (rodio's "audio stream error" messages are silenced by enabling its
///   `tracing` feature in Cargo.toml; a stream that really died can flood
///   thousands of them.)
/// - If the mic never delivers audio (device busy, suspended) its iterator
///   blocks forever. So the mic lives on its own thread and we wait for it
///   with a timeout.
pub fn record_mic(secs: u8, device: Option<&str>) -> Result<Audio, Box<dyn Error>> {
    let (tx, rx) = mpsc::channel::<Result<(Vec<f32>, u16, NonZero<u32>), String>>();
    let device = device.map(str::to_owned);

    // The mic is opened inside the thread because cpal streams can't be
    // moved between threads on every platform.
    thread::spawn(move || {
        let result = (|| -> Result<_, Box<dyn Error>> {
            let builder = match &device {
                Some(query) => MicrophoneBuilder::new().device(find_device(query)?)?,
                None => MicrophoneBuilder::new().default_device()?,
            };
            let mut mic = builder.default_config()?.open_stream()?;
            let channels = mic.channels().get();
            let sample_rate = mic.sample_rate();

            let wanted = secs as usize * sample_rate.get() as usize * channels as usize;
            let mut samples = Vec::with_capacity(wanted);
            let mut last_sample = Instant::now();
            while samples.len() < wanted {
                match mic.next() {
                    Some(sample) => {
                        samples.push(sample);
                        last_sample = Instant::now();
                    }
                    // Stream reported an error; it may well still be running
                    None if last_sample.elapsed() < STALL_TIMEOUT => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    None => break, // it really died; check_length reports it
                }
            }
            Ok((samples, channels, sample_rate))
        })();
        // Box<dyn Error> isn't Send, so pass errors back as text.
        // If the receiver already gave up, there's nobody to tell, so ignore.
        let _ = tx.send(result.map_err(|e| e.to_string()));
    });

    let spinner = ProgressBar::new_spinner(); // draws to stderr
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✔"])
            .template("{spinner:.cyan} {msg}")?,
    );
    spinner.set_message(format!("Recording {secs}s..."));
    spinner.enable_steady_tick(Duration::from_millis(80));

    let received = rx.recv_timeout(Duration::from_secs(secs as u64) + RECORD_GRACE);
    spinner.finish_and_clear();

    let (samples, channels, sample_rate) = match received {
        Ok(Ok(recording)) => recording,
        Ok(Err(e)) => return Err(format!("could not record from microphone: {e}").into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err("microphone stopped delivering audio (timed out). \
                        Check that no other app has exclusive access to it, \
                        or pick another device with --device"
                .into());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err("recording thread crashed".into());
        }
    };

    check_length(samples.len(), channels, sample_rate.get(), secs)?;

    let mono = to_mono(&samples, channels);
    let skip = (sample_rate.get() as usize * TRIM_START_MS as usize / 1000).min(mono.len());
    Ok(Audio {
        mono: mono[skip..].to_vec(),
        sample_rate,
    })
}

/// Fails if the recording is noticeably shorter than requested, which means
/// the audio stream errored partway through.
fn check_length(len: usize, channels: u16, sample_rate: u32, secs: u8) -> Result<(), String> {
    let expected = secs as usize * sample_rate as usize * channels as usize;
    // Allow 5% slack in case the stream hiccups briefly
    if len * 100 < expected * 95 {
        let got_secs = len as f32 / (sample_rate as f32 * channels as f32);
        return Err(format!(
            "recording stopped early: got {got_secs:.1}s of {secs}s because the \
             audio stream stopped. The device may be in use by another program \
             (raw hardware devices are usually held by PipeWire or PulseAudio); \
             try the default device or `--device pipewire`"
        ));
    }
    Ok(())
}

/// Plays audio back through the default output (used by --debug).
pub fn play(audio: &Audio) -> Result<(), Box<dyn Error>> {
    eprintln!("Playing recording...");
    let sink = rodio::DeviceSinkBuilder::open_default_sink()?;
    let player = rodio::Player::connect_new(sink.mixer());
    let mono = NonZero::new(1).expect("1 is non-zero");
    player.append(rodio::buffer::SamplesBuffer::new(
        mono,
        audio.sample_rate,
        audio.mono.clone(),
    ));
    player.sleep_until_end();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_downmix_averages_frames() {
        assert_eq!(to_mono(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(to_mono(&[0.1, 0.2], 1), vec![0.1, 0.2]);
    }

    #[test]
    fn full_length_recording_passes() {
        // 5s of 44.1kHz stereo, plus a few extra samples like real runs
        assert!(check_length(441_033, 2, 44_100, 5).is_ok());
    }

    #[test]
    fn slightly_short_recording_passes() {
        // 97% of the expected length is within the startup slack
        assert!(check_length(427_770, 2, 44_100, 5).is_ok());
    }

    #[test]
    fn recording_cut_off_by_stream_error_fails() {
        // Stream died after ~2s of a 5s recording
        let err = check_length(176_400, 2, 44_100, 5).unwrap_err();
        assert!(err.contains("got 2.0s of 5s"), "got: {err}");
    }

    #[test]
    fn missing_file_has_clear_error() {
        let err = load(Path::new("/definitely/not/here.wav")).err().unwrap();
        assert!(err.to_string().contains("could not open"), "got: {err}");
    }
}
