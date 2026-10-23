//! Writing a voicebank folder that OpenUtau can load.
//!
//! The folder holds the samples as WAV files, an oto.ini indexing them, a character.txt naming the singer,
//! and a character.yaml following [OpenUtau's schema](https://www.schemastore.org/openutau-character.json).

use std::io;
use std::path::Path;

use fs_err as fs;
use thiserror::Error;

use crate::oto::{Encoding, OtoEntry, OtoError, OtoFile};
use crate::quantity::SampleRate;

/// Written into character.yaml.
/// `text_file_encoding` makes OpenUtau read oto.ini as UTF-8 instead of its default, Shift-JIS,
/// so aliases can use characters such as å, ä and ö.
const CHARACTER_YAML: &str = "singer_type: utau\ntext_file_encoding: utf-8\n";

/// One recording and its oto.ini line.
/// The WAV file is written to `entry.file_name`.
#[derive(Debug, Clone)]
pub struct VoicebankSample {
    /// Samples from −1 to 1.
    pub audio: Vec<f64>,
    pub entry: OtoEntry,
}

#[derive(Debug, Clone)]
pub struct Voicebank {
    pub name: String,
    pub sample_rate: SampleRate,
    pub samples: Vec<VoicebankSample>,
}

#[derive(Debug, Error)]
pub enum VoicebankError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("could not write or read a WAV file")]
    Wav(#[from] hound::Error),
    #[error("could not write oto.ini")]
    Oto(#[from] OtoError),
}

impl Voicebank {
    /// Writes 16-bit mono WAV files, a UTF-8 oto.ini, character.txt, and character.yaml.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be written.
    pub fn write<P>(&self, directory: P) -> Result<(), VoicebankError>
    where
        P: AsRef<Path>,
    {
        let directory = directory.as_ref();

        fs::create_dir_all(directory)?;

        let mut oto = OtoFile::new(Encoding::Utf8);

        for sample in &self.samples {
            let path = directory.join(&sample.entry.file_name);

            write_wav(path, &sample.audio, self.sample_rate)?;

            oto.push_entry(sample.entry.clone());
        }

        oto.write(directory.join("oto.ini"))?;

        let character_text = format!("name={}\r\n", self.name);

        fs::write(directory.join("character.txt"), character_text)?;
        fs::write(directory.join("character.yaml"), CHARACTER_YAML)?;

        Ok(())
    }
}

/// Writes 16-bit mono audio, clipping samples outside −1 to 1.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn write_wav<P>(path: P, audio: &[f64], sample_rate: SampleRate) -> Result<(), VoicebankError>
where
    P: AsRef<Path>,
{
    let specification = hound::WavSpec {
        channels: 1,
        sample_rate: sample_rate.hertz(),
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    // Open the file with fs-err and hand it to hound, so errors name the path.
    let file = fs::File::create(path.as_ref())?;
    let mut writer = hound::WavWriter::new(io::BufWriter::new(file), specification)?;

    for &sample in audio {
        let scaled = sample.clamp(-1.0, 1.0) * f64::from(i16::MAX);

        #[expect(
            clippy::cast_possible_truncation,
            reason = "the sample is clamped to the i16 range first"
        )]
        let scaled = scaled.round() as i16;

        writer.write_sample(scaled)?;
    }

    writer.finalize()?;

    Ok(())
}

/// Reads a WAV file as samples from −1 to 1, keeping only the first channel.
///
/// # Errors
///
/// Returns an error when the file cannot be read or is not a valid WAV file.
pub fn read_wav<P>(path: P) -> Result<(Vec<f64>, SampleRate), VoicebankError>
where
    P: AsRef<Path>,
{
    let file = fs::File::open(path.as_ref())?;
    let mut reader = hound::WavReader::new(io::BufReader::new(file))?;

    let specification = reader.spec();
    let channels = usize::from(specification.channels);

    let samples: Vec<f64> = match specification.sample_format {
        hound::SampleFormat::Int => {
            let full_scale = f64::from(1u32 << (specification.bits_per_sample - 1));

            reader
                .samples::<i32>()
                .step_by(channels)
                .map(|sample| sample.map(|value| f64::from(value) / full_scale))
                .collect::<Result<_, _>>()?
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .step_by(channels)
            .map(|sample| sample.map(f64::from))
            .collect::<Result<_, _>>()?,
    };

    let sample_rate = SampleRate::from_hertz(specification.sample_rate);

    Ok((samples, sample_rate))
}
