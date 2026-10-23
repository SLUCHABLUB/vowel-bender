use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use fn_error_context::context;
use fs_err as fs;
use vowel_bender::oto::{Encoding, OtoFile};
use vowel_bender::quantity::{Frequency, Interval};
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
        /// The default is near the mean pitch of the speakers whose formants the bank uses.
        #[arg(long, default_value = "D3", value_parser = note_to_frequency)]
        pitch: Frequency,
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
#[context("parsing `{note}` as a note")]
fn note_to_frequency(note: &str) -> anyhow::Result<Frequency> {
    let semitone_in_octave = match note
        .chars()
        .next()
        .context("parsing the letter part")?
        .to_ascii_uppercase()
    {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        letter => bail!("invalid note letter: {letter}"),
    };

    let rest = &note[1..];

    let (accidental, octave) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('b') => (-1, &rest[1..]),
        _ => (0, rest),
    };

    let octave: i32 = octave.parse().context("parsing the octave")?;

    let semitones_from_a4 = (octave - 4) * 12 + semitone_in_octave + accidental - 9;
    let interval_from_a4 = Interval::from_semitones(f64::from(semitones_from_a4));

    let frequency = Frequency::from_hertz(440.0) * interval_from_a4;

    Ok(frequency)
}

fn main() -> anyhow::Result<()> {
    match Arguments::parse().command {
        Command::SynthBank { output, pitch } => {
            let settings = SyntheticBankSettings {
                base_fundamental_frequency: pitch,
                ..SyntheticBankSettings::default()
            };

            let bank = synthetic::generate(&settings);

            bank.write(&output)?;

            println!(
                "Wrote {} samples around {pitch:.1} to {}",
                bank.voicebank.samples.len(),
                output.display()
            );
        }
        Command::OtoCheck { path, encoding } => {
            let original = fs::read(&path)?;

            let oto = OtoFile::parse(&original, encoding.map(Encoding::from));
            let byte_order_mark = if oto.has_byte_order_mark() {
                " with byte order mark"
            } else {
                ""
            };

            println!("Encoding: {}{byte_order_mark}", oto.encoding());
            println!("Lines: {}", oto.lines().len());
            println!("Entries: {}", oto.entries().count());

            let unparsed: Vec<_> = oto.unparsed_lines().collect();

            println!("Unparsed lines: {}", unparsed.len());

            for (line_number, line) in unparsed {
                let text = String::from_utf8_lossy(line.original_bytes().unwrap_or_default());
                let reason = line
                    .parse_error()
                    .map(ToString::to_string)
                    .unwrap_or_default();

                println!("  {line_number}: {text}");
                println!("    {reason}");
            }

            let identical = oto.to_bytes().is_ok_and(|written| written == original);

            if identical {
                println!("Round trip identical");
            } else {
                bail!("Round trip failed");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::note_to_frequency;

    #[test]
    fn note_names_convert_to_frequency() {
        let close = |note: &str, expected: f64| {
            let frequency = note_to_frequency(note).unwrap();

            (frequency.hertz() - expected).abs() < 0.01
        };
        assert!(close("A4", 440.0));
        assert!(close("A3", 220.0));
        assert!(close("C4", 261.63));
        assert!(close("C#4", 277.18));
        assert!(close("Bb3", 233.08));
        assert!(note_to_frequency("H2").is_err());
        assert!(note_to_frequency("A").is_err());
    }
}
