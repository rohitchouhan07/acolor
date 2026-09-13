use std::error::Error;
use rodio::microphone::{Input, available_inputs};

pub fn print_input_devices() -> Result<(), Box<dyn Error>> {
    let inputs = get_input_devices()?; 
    for (i, input) in inputs.iter().enumerate() {
        println!("Input {}: {}", i, input);
    }

    Ok(())
}

pub fn get_input_devices() -> Result<Vec<Input>, Box<dyn Error>> {
    let inputs = available_inputs()?;
    Ok(inputs)
}

