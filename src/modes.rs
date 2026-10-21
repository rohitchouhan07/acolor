pub mod inputs;
pub mod properties;

use std::{error::Error,num::NonZero, time::Duration};
use crate::colors;
use rodio::{Source, microphone::MicrophoneBuilder};
use indicatif::{ProgressBar, ProgressStyle};

use crate::{RunOpts};
pub fn check_inputs_mode() -> Result<(), Box<dyn Error>> {
    inputs::print_input_devices()?;
    Ok(())
}

fn trim_start(samples: &[f32], sample_rate: NonZero<u32>, channels: u16, ms: u32) -> &[f32] {
    let skip = (sample_rate.get() as u64 * ms as u64 / 1000) as usize * channels as usize;

    if skip < samples.len() {
        &samples[skip..]
    } else {
        samples // recording shorter than the trim window, so leave it alone
    }
}

pub fn main_mode(opts: RunOpts) -> Result<(), Box<dyn Error>> {
    let mic = MicrophoneBuilder::new()
    .default_device()?
    .default_config()?
    .open_stream()?;

    let channels = mic.channels();
    let sample_rate = mic.sample_rate();

    if opts.debug == true {
        println!("Run options: {:?}", opts);
        println!("{channels}");
        println!("{sample_rate}");
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );

    pb.set_message("Sampling audio...");
    pb.enable_steady_tick(Duration::from_millis(80));

    // NOTE: Something funny happens here, where it errors out, need to fix it
    let recording = mic.take_duration(Duration::from_secs(opts.sample_time as u64));
    let samples: Vec<f32> = recording.collect();
    
    pb.finish_and_clear();
    pb.finish_with_message("✔ Sampling complete!");

    let trimmed = trim_start(&samples, sample_rate, 1, 500);
    let palette = colors::generate_palette(trimmed, sample_rate, &opts);
     
    println!("Generated palette:");
    for (r, g, b) in &palette {
        println!("  rgb({}, {}, {})", r, g, b);
    }
  
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
