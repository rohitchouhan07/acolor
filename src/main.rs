use clap::{Arg, Command, builder, ValueEnum};
use std::{error::Error, process};
pub mod modes;

#[derive(ValueEnum, Clone, Debug)]
enum Mode {
    CheckInputs,
    Debug,
}

#[derive(Debug)]
pub struct RunOpts {
    pub debug: bool,
    pub sample_time: u8,
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
                                            .required(false)
                                            .value_parser(builder::EnumValueParser::<Mode>::new())
                                    )
                                    .arg(
                                        Arg::new("sample")
                                            .short('s')
                                            .help("Sample time in seconds")
                                            .required(false)
                                            .value_parser(clap::value_parser!(u8).range(5..=30))
                                            .default_value("5")
                                    );
    
    let matches = cmd.get_matches();
    let sample_time: u8 = *matches.get_one::<u8>("sample").unwrap();
    
    match matches.get_one::<Mode>("mode") {
           Some(mode) => match &mode {
                            Mode::CheckInputs => crate::modes::check_inputs_mode()?,
                            Mode::Debug => crate::modes::main_mode(RunOpts 
                                                            {
                                                                debug: true,
                                                                sample_time,
                                                            })?,
                                    },
           None => crate::modes::main_mode(RunOpts
               {
                   debug: false,
                   sample_time,
               })?,
    };

    Ok(())
}

