use clap::{Arg, Command, builder, ValueEnum};
use std::{error::Error, process};
pub mod modes;

#[derive(ValueEnum, Clone, Debug)]
enum Mode {
    Inputs,
    Debug,
}
  
fn main() {
    println!("AcoloR - Ambient Color");
        
    if let Err(e) = run() {
        println!("Error: {e}");
        process::exit(1);
    }

}

fn run() -> Result<(), Box<dyn Error>> {
    
    let cmd = Command::new("acolor").author("rc")
                                    .version("0.1.0")
                                    .about("Color palette generator based on sound")
                                    .arg(
                                        Arg::new("mode")
                                            .short('m')
                                            .help("Choose a mode")
                                            .required(true)
                                            .value_parser(builder::EnumValueParser::<Mode>::new())
                                    );
    let matches = cmd.get_matches();
    let mode = matches.get_one::<Mode>("mode").unwrap();

    match mode {
        Mode::Inputs => crate::modes::check_input_devices(),
        Mode::Debug => println!("Running in debug mode."),
    }
    Ok(())
}
