mod audio;
mod colors;
mod output;

use std::{
    error::Error,
    io::{IsTerminal, stdout},
    path::PathBuf,
    process,
};

use clap::{Parser, Subcommand};

use crate::output::Format;

/// Generate color palettes from sound.
///
/// Analyzes an audio file (or the microphone) and turns its notes,
/// loudness and texture into a palette.
#[derive(Parser, Debug)]
#[command(name = "acolor", version)]
#[command(args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    analyze: AnalyzeArgs,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List available input devices
    Devices,
}

#[derive(clap::Args, Debug)]
struct AnalyzeArgs {
    /// Audio file to analyze (wav, mp3, flac, ogg, m4a), or - to read from
    /// stdin. Records from the microphone if omitted.
    file: Option<PathBuf>,

    /// Number of colors in the palette
    #[arg(short = 'n', long, default_value_t = 5,
          value_parser = clap::value_parser!(u8).range(1..=12))]
    colors: u8,

    /// Output format [default: term in a terminal, hex when piped]
    #[arg(short, long, value_enum)]
    format: Option<Format>,

    /// Also save a palette card image to this path
    #[arg(short, long, value_name = "PNG")]
    output: Option<PathBuf>,

    /// Microphone recording length in seconds
    #[arg(short, long, default_value_t = 5, conflicts_with = "file",
          value_parser = clap::value_parser!(u8).range(5..=30))]
    seconds: u8,

    /// Microphone to record from, by index or name (see `acolor devices`)
    #[arg(short, long, conflicts_with = "file")]
    device: Option<String>,

    /// Print analysis details, and play back microphone recordings
    #[arg(long)]
    debug: bool,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Command::Devices) => audio::print_devices(),
        None => analyze(cli.analyze),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        process::exit(1);
    }
}

fn analyze(args: AnalyzeArgs) -> Result<(), Box<dyn Error>> {
    let from_mic = args.file.is_none();
    let audio = match &args.file {
        Some(path) => audio::load(path)?,
        None => audio::record_mic(args.seconds, args.device.as_deref())?,
    };

    let palette =
        colors::generate_palette(&audio.mono, audio.sample_rate, args.colors as usize, args.debug)?;

    // Palette goes to stdout so it can be piped; everything else goes to stderr
    let format = args.format.unwrap_or_else(default_format);
    print!("{}", output::render(&palette, format));

    if let Some(path) = &args.output {
        output::save_palette_card(&palette, path)?;
        eprintln!("Saved {}", path.display());
    }

    if args.debug && from_mic {
        audio::play(&audio)?;
    }
    Ok(())
}

/// Color swatches for humans, plain hex for scripts.
/// Respects the NO_COLOR convention (https://no-color.org).
fn default_format() -> Format {
    if stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
        Format::Term
    } else {
        Format::Hex
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn file_and_mic_options_conflict() {
        assert!(Cli::try_parse_from(["acolor", "song.mp3", "-s", "10"]).is_err());
        assert!(Cli::try_parse_from(["acolor", "song.mp3", "-d", "1"]).is_err());
        assert!(Cli::try_parse_from(["acolor", "song.mp3", "-n", "8", "-f", "json"]).is_ok());
    }

    #[test]
    fn devices_subcommand_parses() {
        let cli = Cli::try_parse_from(["acolor", "devices"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Devices)));
    }
}
