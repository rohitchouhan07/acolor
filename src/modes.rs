pub mod inputs;
use std::{error::Error, time::Duration};
use rodio::{Source, microphone::MicrophoneBuilder};

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

    let recording = mic.take_duration(Duration::from_secs(opts.sample_time as u64));
    
    for sample in recording {
        println!("{}", sample);
    }

    Ok(())
}
