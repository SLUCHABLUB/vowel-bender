//! Writing a voicebank folder that OpenUtau can load.

use std::fs;
use std::io;
use std::path::Path;

use crate::oto::{Encoding, OtoEntry, OtoFile};

/// One recording and its oto.ini line.
/// The wav is written to `entry.file_name`.
#[derive(Debug, Clone)]
pub struct VoicebankSample {
    pub audio: Vec<f64>,
    pub entry: OtoEntry,
}

#[derive(Debug, Clone)]
pub struct Voicebank {
    pub name: String,
    pub sample_rate: u32,
    pub samples: Vec<VoicebankSample>,
}

impl Voicebank {
    /// Writes 16-bit mono WAV files, a UTF-8 oto.ini, character.txt, and a character.yaml that tells OpenUtau to read them as UTF-8.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be written.
    pub fn write(&self, directory: &Path) -> io::Result<()> {
        fs::create_dir_all(directory)?;

        let mut oto = OtoFile::new(Encoding::Utf8);
        for sample in &self.samples {
            write_wav(
                &directory.join(&sample.entry.file_name),
                &sample.audio,
                self.sample_rate,
            )?;
            oto.push_entry(sample.entry.clone());
        }
        oto.write(&directory.join("oto.ini"))
            .map_err(io::Error::other)?;

        fs::write(
            directory.join("character.txt"),
            format!("name={}\r\n", self.name),
        )?;
        fs::write(
            directory.join("character.yaml"),
            "singer_type: utau\ntext_file_encoding: utf-8\n",
        )?;
        Ok(())
    }
}

/// Writes 16-bit mono audio, clipping samples outside −1 to 1.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn write_wav(path: &Path, audio: &[f64], sample_rate: u32) -> io::Result<()> {
    let specification = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, specification).map_err(io::Error::other)?;
    for &sample in audio {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the sample is clamped to the i16 range first"
        )]
        let scaled = (sample.clamp(-1.0, 1.0) * f64::from(i16::MAX)).round() as i16;
        writer.write_sample(scaled).map_err(io::Error::other)?;
    }
    writer.finalize().map_err(io::Error::other)
}

/// Reads a wav as samples from −1 to 1, keeping only the first channel.
/// Returns the samples and the sample rate.
///
/// # Errors
///
/// Returns an error when the file cannot be read or is not a valid wav.
pub fn read_wav(path: &Path) -> io::Result<(Vec<f64>, u32)> {
    let mut reader = hound::WavReader::open(path).map_err(io::Error::other)?;
    let specification = reader.spec();
    let channels = usize::from(specification.channels);
    let samples: Vec<f64> = match specification.sample_format {
        hound::SampleFormat::Int => {
            let full_scale = f64::from(1u32 << (specification.bits_per_sample - 1));
            reader
                .samples::<i32>()
                .step_by(channels)
                .map(|sample| sample.map(|value| f64::from(value) / full_scale))
                .collect::<Result<_, _>>()
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .step_by(channels)
            .map(|sample| sample.map(f64::from))
            .collect::<Result<_, _>>(),
    }
    .map_err(io::Error::other)?;
    Ok((samples, specification.sample_rate))
}
