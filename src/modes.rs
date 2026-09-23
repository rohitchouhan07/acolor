pub mod inputs;
pub mod properties;

use std::{error::Error, time::Duration};
use crate::colors;
use rodio::{Source, microphone::MicrophoneBuilder};
use indicatif::{ProgressBar, ProgressStyle};

use crate::{RunOpts};
pub fn check_inputs_mode() -> Result<(), Box<dyn Error>> {
    inputs::print_input_devices()?;
    Ok(())
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

    // NOTE: Something funny happens here, where it errors out, need to fix it
    pb.set_message("Sampling audio...");
    pb.enable_steady_tick(Duration::from_millis(80));

    let recording = mic.take_duration(Duration::from_secs(opts.sample_time as u64));
    let samples: Vec<f32> = recording.collect();

    pb.finish_with_message("✔ Sampling complete!");
    
    let palette = colors::generate_palette(&samples, sample_rate, 5);
     
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
