//! A synthetic Japanese CV voicebank rendered from known formants.
//!
//! Every vowel is a glottal pulse train through a cascade of resonators with known settings,
//! so later stages can be checked against ground truth.
//! Consonants are crude on purpose, since the tool passes them through untouched.
//! Like a real Japanese bank, it has shi, chi, tsu and fu in place of si, ti, tu and hu.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::filter::{Antiresonator, Cascade, Resonator};
use crate::oto::OtoEntry;
use crate::voicebank::{Voicebank, VoicebankSample};

/// The file, next to the bank's oto.ini, that records what the bank was rendered from.
pub const TRUTH_FILE_NAME: &str = "synthetic-truth.toml";

/// F1 to F4 of one vowel, in Hz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VowelFormants {
    pub formants: [f64; 4],
    pub bandwidths: [f64; 4],
}

impl VowelFormants {
    fn pairs(&self) -> Vec<(f64, f64)> {
        self.formants
            .iter()
            .copied()
            .zip(self.bandwidths.iter().copied())
            .collect()
    }
}

/// Typical adult male values for the five Japanese vowels.
/// Japanese u is unrounded and compressed, so its F2 sits higher than a rounded u's.
#[must_use]
pub fn japanese_vowels() -> Vec<(&'static str, VowelFormants)> {
    let vowel = |formants, bandwidths| VowelFormants {
        formants,
        bandwidths,
    };

    // TODO: Where do these values come from?
    vec![
        (
            "a",
            vowel([800.0, 1250.0, 2550.0, 3550.0], [90.0, 100.0, 140.0, 200.0]),
        ),
        (
            "i",
            vowel([300.0, 2250.0, 3000.0, 3700.0], [60.0, 110.0, 160.0, 220.0]),
        ),
        (
            "u",
            vowel([350.0, 1350.0, 2400.0, 3500.0], [65.0, 100.0, 140.0, 200.0]),
        ),
        (
            "e",
            vowel([500.0, 1900.0, 2600.0, 3600.0], [75.0, 100.0, 150.0, 210.0]),
        ),
        (
            "o",
            vowel([500.0, 850.0, 2500.0, 3500.0], [75.0, 90.0, 140.0, 200.0]),
        ),
    ]
}

#[derive(Debug, Clone, Copy)]
enum ConsonantSound {
    /// Optional silence, then filtered noise.
    Noise {
        closure: f64,
        duration: f64,
        center_frequency: f64,
        bandwidth: f64,
        level: f64,
    },
    /// The glottal source through a low resonance and an antiresonance.
    Nasal {
        duration: f64,
        antiresonance_frequency: f64,
    },
}

impl ConsonantSound {
    /// Length in seconds, including any closure.
    fn duration(self) -> f64 {
        match self {
            ConsonantSound::Noise {
                closure, duration, ..
            } => closure + duration,
            ConsonantSound::Nasal { duration, .. } => duration,
        }
    }
}

fn noise(
    closure: f64,
    duration: f64,
    center_frequency: f64,
    bandwidth: f64,
    level: f64,
) -> ConsonantSound {
    ConsonantSound::Noise {
        closure,
        duration,
        center_frequency,
        bandwidth,
        level,
    }
}

fn consonant_sound(consonant: &str) -> Option<ConsonantSound> {
    // TODO: Where do these values come from?
    Some(match consonant {
        "k" => noise(0.04, 0.03, 2000.0, 1200.0, 0.3),
        "s" => noise(0.0, 0.12, 6000.0, 2500.0, 0.2),
        "sh" => noise(0.0, 0.12, 3500.0, 1500.0, 0.25),
        "t" => noise(0.04, 0.02, 4000.0, 2000.0, 0.3),
        "ch" => noise(0.04, 0.08, 3500.0, 1500.0, 0.25),
        "ts" => noise(0.04, 0.08, 6000.0, 2500.0, 0.2),
        "h" => noise(0.0, 0.08, 1500.0, 2500.0, 0.1),
        "f" => noise(0.0, 0.08, 1200.0, 2500.0, 0.08),
        "n" => ConsonantSound::Nasal {
            duration: 0.07,
            antiresonance_frequency: 1800.0,
        },
        "m" => ConsonantSound::Nasal {
            duration: 0.07,
            antiresonance_frequency: 1000.0,
        },
        _ => return None,
    })
}

/// The consonant Japanese uses before a vowel in each row, or `""` for a bare vowel.
fn japanese_consonant(row: &'static str, vowel: &str) -> &'static str {
    match (row, vowel) {
        ("s", "i") => "sh",
        ("t", "i") => "ch",
        ("t", "u") => "ts",
        ("h", "u") => "f",
        _ => row,
    }
}

const CONSONANT_ROWS: [&str; 7] = ["", "k", "s", "t", "n", "h", "m"];

#[derive(Debug, Clone)]
pub struct SyntheticBankSettings {
    pub name: String,
    pub sample_rate: u32,
    /// The centre of the pitch band the samples spread across, in Hz.
    pub base_fundamental_frequency: f64,
    /// Width of the pitch band in semitones.
    pub pitch_spread_semitones: f64,
    pub seed: u64,
}

impl Default for SyntheticBankSettings {
    fn default() -> Self {
        Self {
            name: "Synthetic CV".to_string(),
            sample_rate: 44_100,
            base_fundamental_frequency: 220.0,
            pitch_spread_semitones: 4.0,
            seed: 1,
        }
    }
}

/// What one sample was rendered from.
/// Times are in milliseconds from the start of the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleTruth {
    pub file_name: String,
    pub alias: String,
    pub consonant: String,
    pub vowel: String,
    pub fundamental_frequency: f64,
    pub consonant_start: f64,
    pub vowel_start: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticTruth {
    pub sample_rate: u32,
    pub base_fundamental_frequency: f64,
    pub vowels: BTreeMap<String, VowelFormants>,
    pub samples: Vec<SampleTruth>,
}

impl SyntheticTruth {
    /// Reads a truth file written by [`SyntheticBank::write`].
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or is not a valid truth file.
    pub fn read(path: &Path) -> io::Result<Self> {
        toml::from_str(&fs::read_to_string(path)?).map_err(io::Error::other)
    }
}

#[derive(Debug, Clone)]
pub struct SyntheticBank {
    pub voicebank: Voicebank,
    pub truth: SyntheticTruth,
}

impl SyntheticBank {
    /// Writes the voicebank and its truth file into `directory`.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be written.
    pub fn write(&self, directory: &Path) -> io::Result<()> {
        self.voicebank.write(directory)?;

        let truth = toml::to_string(&self.truth).map_err(io::Error::other)?;
        fs::write(directory.join(TRUTH_FILE_NAME), truth)?;

        Ok(())
    }
}

/// Deterministic white noise from −1 to 1 (xorshift64*).
#[derive(Debug, Clone)]
pub struct NoiseGenerator {
    state: u64,
}

impl NoiseGenerator {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        // A zero state would stay zero forever.
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    pub fn next_sample(&mut self) -> f64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let value = self.state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // The top 52 bits become the mantissa of a number from 1 to 2.
        let one_to_two = f64::from_bits(0x3FF0_0000_0000_0000 | (value >> 12));
        (one_to_two - 1.0) * 2.0 - 1.0
    }
}

/// Glottal flow derivative from Rosenberg pulses, with slight period jitter.
struct GlottalSource {
    sample_rate: f64,
    fundamental_frequency: f64,
    period_frequency: f64,
    phase: f64,
    previous_flow: f64,
    jitter: NoiseGenerator,
}

impl GlottalSource {
    /// Share of each period the folds spend opening, then closing.
    const OPENING: f64 = 0.4;
    const CLOSING: f64 = 0.16;
    /// Largest relative change of one period's length.
    const JITTER: f64 = 0.005;

    fn new(fundamental_frequency: f64, sample_rate: f64, seed: u64) -> Self {
        Self {
            sample_rate,
            fundamental_frequency,
            period_frequency: fundamental_frequency,
            phase: 0.0,
            previous_flow: 0.0,
            jitter: NoiseGenerator::new(seed),
        }
    }

    fn flow(phase: f64) -> f64 {
        use std::f64::consts::PI;
        if phase < Self::OPENING {
            0.5 * (1.0 - (PI * phase / Self::OPENING).cos())
        } else if phase < Self::OPENING + Self::CLOSING {
            (PI * (phase - Self::OPENING) / (2.0 * Self::CLOSING)).cos()
        } else {
            0.0
        }
    }

    fn next_sample(&mut self) -> f64 {
        let flow = Self::flow(self.phase);
        // Differentiating the flow models radiation from the lips.
        let derivative = flow - self.previous_flow;
        self.previous_flow = flow;
        self.phase += self.period_frequency / self.sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.period_frequency =
                self.fundamental_frequency * (1.0 + Self::JITTER * self.jitter.next_sample());
        }
        derivative
    }
}

/// Timing of one rendered sample, in seconds.
struct Layout {
    consonant_start: f64,
    vowel_start: f64,
    vowel_end: f64,
    length: f64,
}

const LEADING_SILENCE: f64 = 0.1;
const TRAILING_SILENCE: f64 = 0.1;
const VOWEL_DURATION: f64 = 0.8;
const VOWEL_ATTACK: f64 = 0.03;
const VOWEL_RELEASE: f64 = 0.08;
const CROSSFADE: f64 = 0.015;
const VOWEL_LEVEL: f64 = 0.5;
const NASAL_LEVEL: f64 = 0.2;

impl Layout {
    fn new(consonant: Option<ConsonantSound>) -> Self {
        let consonant_start = LEADING_SILENCE;
        let vowel_start = consonant_start + consonant.map_or(0.0, ConsonantSound::duration);
        let vowel_end = vowel_start + VOWEL_DURATION;
        Self {
            consonant_start,
            vowel_start,
            vowel_end,
            length: vowel_end + TRAILING_SILENCE,
        }
    }

    fn oto_entry(&self, file_name: String, alias: String, has_consonant: bool) -> OtoEntry {
        let milliseconds = |seconds: f64| (seconds * 1000.0).round();

        let offset = milliseconds(self.consonant_start) - 30.0;
        let preutterance = milliseconds(self.vowel_start) - offset;

        OtoEntry {
            file_name,
            alias,
            offset,
            consonant: preutterance + 50.0,
            cutoff: -(milliseconds(self.vowel_end - VOWEL_RELEASE) - offset),
            preutterance,
            overlap: if has_consonant { 20.0 } else { 10.0 },
        }
    }
}

/// Gain from 0 to 1 that rises linearly over `attack` from `start` and falls over `release` until `end`.
fn envelope(time: f64, start: f64, attack: f64, end: f64, release: f64) -> f64 {
    if time < start || time >= end {
        0.0
    } else {
        ((time - start) / attack)
            .min((end - time) / release)
            .min(1.0)
    }
}

fn normalize(signal: &mut [f64], level: f64) {
    let peak = signal
        .iter()
        .copied()
        .map(f64::abs)
        .reduce(f64::max)
        .unwrap_or(0.0);

    if peak > 0.0 {
        for sample in signal {
            *sample *= level / peak;
        }
    }
}

fn render(
    vowel: &VowelFormants,
    consonant: Option<ConsonantSound>,
    layout: &Layout,
    fundamental_frequency: f64,
    sample_rate: u32,
    seed: u64,
) -> Vec<f64> {
    let rate = f64::from(sample_rate);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the length is positive and a few seconds long"
    )]
    let sample_count = (layout.length * rate).round() as usize;
    #[expect(
        clippy::cast_precision_loss,
        reason = "sample indices stay far below 2^52"
    )]
    let time = |index: usize| index as f64 / rate;

    let mut source = GlottalSource::new(fundamental_frequency, rate, seed);
    let glottal: Vec<_> = (0..sample_count).map(|_| source.next_sample()).collect();

    // A consonant hands over to the vowel with a crossfade, a bare vowel starts with a plain attack.
    let vowel_attack = if consonant.is_some() {
        CROSSFADE
    } else {
        VOWEL_ATTACK
    };

    let mut tract = Cascade::new(&vowel.pairs(), rate);
    let mut output: Vec<f64> = glottal
        .iter()
        .enumerate()
        .map(|(index, &sample)| {
            let gain = envelope(
                time(index),
                layout.vowel_start,
                vowel_attack,
                layout.vowel_end,
                VOWEL_RELEASE,
            );
            tract.process_sample(sample) * gain
        })
        .collect();
    normalize(&mut output, VOWEL_LEVEL);

    let consonant_end = layout.vowel_start + CROSSFADE;
    let consonant_part: Vec<f64> = match consonant {
        None => Vec::new(),
        Some(ConsonantSound::Noise {
            closure,
            center_frequency,
            bandwidth,
            level,
            ..
        }) => {
            let mut generator = NoiseGenerator::new(seed.wrapping_add(1));
            let mut shaping = Resonator::new(center_frequency, bandwidth, rate);
            let release_start = layout.consonant_start + closure;
            let mut part: Vec<f64> = (0..sample_count)
                .map(|index| {
                    let gain =
                        envelope(time(index), release_start, 0.005, consonant_end, CROSSFADE);
                    shaping.process_sample(generator.next_sample()) * gain
                })
                .collect();
            normalize(&mut part, level);
            part
        }
        Some(ConsonantSound::Nasal {
            antiresonance_frequency,
            ..
        }) => {
            let mut murmur = Resonator::new(250.0, 100.0, rate);
            let mut antiresonance = Antiresonator::new(antiresonance_frequency, 200.0, rate);

            let mut part: Vec<f64> = glottal
                .iter()
                .enumerate()
                .map(|(index, &sample)| {
                    let gain = envelope(
                        time(index),
                        layout.consonant_start,
                        0.01,
                        consonant_end,
                        CROSSFADE,
                    );
                    antiresonance.process_sample(murmur.process_sample(sample)) * gain
                })
                .collect();
            normalize(&mut part, NASAL_LEVEL);

            part
        }
    };

    for (sample, consonant_sample) in output.iter_mut().zip(consonant_part) {
        *sample += consonant_sample;
    }

    output
}

/// Spreads samples across the pitch band so each vowel's samples cover it evenly.
fn fundamental_frequency(settings: &SyntheticBankSettings, sample_index: usize) -> f64 {
    const GOLDEN_RATIO_CONJUGATE: f64 = 0.618_033_988_749_895;
    #[expect(
        clippy::cast_precision_loss,
        reason = "sample indices stay far below 2^52"
    )]
    let position = (sample_index as f64 * GOLDEN_RATIO_CONJUGATE).fract() - 0.5;
    settings.base_fundamental_frequency
        * 2.0_f64.powf(position * settings.pitch_spread_semitones / 12.0)
}

#[must_use]
pub fn generate(settings: &SyntheticBankSettings) -> SyntheticBank {
    let vowels = japanese_vowels();
    let mut samples = Vec::new();
    let mut sample_truths = Vec::new();

    for row in CONSONANT_ROWS {
        for (vowel_id, vowel) in &vowels {
            let consonant_id = japanese_consonant(row, vowel_id);
            let consonant = consonant_sound(consonant_id);
            let alias = format!("{consonant_id}{vowel_id}");
            let file_name = format!("{alias}.wav");
            let sample_index = samples.len();
            let fundamental_frequency = fundamental_frequency(settings, sample_index);
            let layout = Layout::new(consonant);
            let seed = settings
                .seed
                .wrapping_mul(1000)
                .wrapping_add(sample_index as u64);

            samples.push(VoicebankSample {
                audio: render(
                    vowel,
                    consonant,
                    &layout,
                    fundamental_frequency,
                    settings.sample_rate,
                    seed,
                ),
                entry: layout.oto_entry(file_name.clone(), alias.clone(), consonant.is_some()),
            });
            sample_truths.push(SampleTruth {
                file_name,
                alias,
                consonant: consonant_id.to_string(),
                vowel: vowel_id.to_string(),
                fundamental_frequency,
                consonant_start: layout.consonant_start * 1000.0,
                vowel_start: layout.vowel_start * 1000.0,
            });
        }
    }

    SyntheticBank {
        voicebank: Voicebank {
            name: settings.name.clone(),
            sample_rate: settings.sample_rate,
            samples,
        },
        truth: SyntheticTruth {
            sample_rate: settings.sample_rate,
            base_fundamental_frequency: settings.base_fundamental_frequency,
            vowels: vowels
                .into_iter()
                .map(|(id, formants)| (id.to_string(), formants))
                .collect(),
            samples: sample_truths,
        },
    }
}
