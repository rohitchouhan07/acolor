pub mod inputs;
use std::{error::Error, time::Duration};
use rodio::{Source, microphone::MicrophoneBuilder};
use indicatif::{ProgressBar, ProgressStyle};

use crate::{RunOpts, modes};

pub fn check_inputs_mode() -> Result<(), Box<dyn Error>> {
    modes::inputs::print_input_devices()?;
    Ok(())
}

pub fn main_mode(opts: RunOpts) -> Result<(), Box<dyn Error>> {
    if opts.debug == true {
        println!("Run options: {:?}", opts);
    }

    let mic = MicrophoneBuilder::new()
    .default_device()?
    .default_config()?
    .open_stream()?;

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
    
    if opts.debug == true {
        println!("{:?}", samples);
    }

    
    // RMS (root mean square) gives you overall loudness
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    
    // Peak amplitude (loudest single sample)
    let peak = samples.iter().fold(0.0_f32, |max, &s| max.max(s.abs()));
    
    println!("RMS: {}, Peak: {}", rms, peak);

    Ok(())
}
