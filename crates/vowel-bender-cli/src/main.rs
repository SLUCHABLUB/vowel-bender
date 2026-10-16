use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use vowel_bender::oto::{Encoding, OtoFile};
use vowel_bender::synthetic::{self, SyntheticBankSettings};

#[derive(Parser)]
#[command(
    name = "vowel-bender",
    about = "Moves the vowels in a voicebank toward the vowels in another language"
)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Writes a synthetic Japanese CV voicebank rendered from known formants.
    SynthBank {
        /// Folder to write the voicebank into.
        #[arg(long)]
        output: PathBuf,
        /// Centre of the pitch band, as a note name such as A3 or C#4.
        #[arg(long, default_value = "A3", value_parser = note_to_frequency)]
        pitch: f64,
    },
    /// Parses an oto.ini and checks that writing it back gives identical bytes.
    OtoCheck {
        path: PathBuf,
        /// Encoding to read the file in; detected when omitted.
        #[arg(long)]
        encoding: Option<EncodingArgument>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum EncodingArgument {
    ShiftJis,
    Utf8,
}

impl From<EncodingArgument> for Encoding {
    fn from(argument: EncodingArgument) -> Self {
        match argument {
            EncodingArgument::ShiftJis => Encoding::ShiftJis,
            EncodingArgument::Utf8 => Encoding::Utf8,
        }
    }
}

/// Converts a note name such as A4, C#4 or Bb3 to Hz, with A4 at 440 Hz.
fn note_to_frequency(note: &str) -> Result<f64, String> {
    let error = || format!("not a note name: {note}");
    let mut characters = note.chars();
    let semitone_in_octave = match characters.next().ok_or_else(error)?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return Err(error()),
    };
    let rest = characters.as_str();
    let (accidental, octave) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('b') => (-1, &rest[1..]),
        _ => (0, rest),
    };
    let octave: i32 = octave.parse().map_err(|_| error())?;
    let semitones_from_a4 = (octave - 4) * 12 + semitone_in_octave + accidental - 9;
    Ok(440.0 * 2.0_f64.powf(f64::from(semitones_from_a4) / 12.0))
}

fn main() -> ExitCode {
    match Arguments::parse().command {
        Command::SynthBank { output, pitch } => {
            let settings = SyntheticBankSettings {
                base_fundamental_frequency: pitch,
                ..SyntheticBankSettings::default()
            };
            let bank = synthetic::generate(&settings);
            if let Err(error) = bank.write(&output) {
                eprintln!("could not write {}: {error}", output.display());
                return ExitCode::FAILURE;
            }
            println!(
                "Wrote {} samples around {pitch:.1} Hz to {}",
                bank.voicebank.samples.len(),
                output.display()
            );
            ExitCode::SUCCESS
        }
        Command::OtoCheck { path, encoding } => {
            let original = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    eprintln!("could not read {}: {error}", path.display());
                    return ExitCode::FAILURE;
                }
            };
            let oto = OtoFile::parse(&original, encoding.map(Encoding::from));
            let byte_order_mark = if oto.has_byte_order_mark() {
                " with byte order mark"
            } else {
                ""
            };
            println!("Encoding: {}{byte_order_mark}", oto.encoding());
            println!("Lines: {}", oto.lines().len());
            println!("Entries: {}", oto.entries().count());
            let unparsed = oto.unparsed_line_indices();
            println!("Unparsed lines: {}", unparsed.len());
            for line_index in unparsed {
                println!(
                    "  {}: {}",
                    line_index + 1,
                    String::from_utf8_lossy(oto.lines()[line_index].raw())
                );
            }
            let identical = oto.to_bytes().is_ok_and(|written| written == original);
            println!(
                "Round trip identical: {}",
                if identical { "yes" } else { "no" }
            );
            if identical {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::note_to_frequency;

    #[test]
    fn note_names_convert_to_frequency() {
        let close =
            |note: &str, expected: f64| (note_to_frequency(note).unwrap() - expected).abs() < 0.01;
        assert!(close("A4", 440.0));
        assert!(close("A3", 220.0));
        assert!(close("C4", 261.63));
        assert!(close("C#4", 277.18));
        assert!(close("Bb3", 233.08));
        assert!(note_to_frequency("H2").is_err());
        assert!(note_to_frequency("A").is_err());
    }
}
