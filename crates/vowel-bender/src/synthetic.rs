//! A synthetic Japanese CV voicebank rendered from known formants.
//!
//! Every vowel is a glottal pulse train through a cascade of resonators with known settings,
//! so later stages can be checked against ground truth.
//! Consonants are crude on purpose, since the tool passes them through untouched.
//! Like a real Japanese bank, it has shi, chi, tsu and fu in place of si, ti, tu and hu.
//!
//! Values come from these sources, and each constant says which one it uses or that it was picked by hand:
//!
//! - Nakagawa, Shirakata, Yamao and Sakai (1980),
//!   [Differences in Feature Parameters of Japanese Vowels with Sex and Age](https://repository.kulib.kyoto-u.ac.jp/dspace/bitstream/2433/52557/1/soa014_033.pdf),
//!   Studia Phonologica 14, Table 1: formant frequencies of the five vowels for groups of Japanese speakers.
//! - Klatt (1980), [Software for a cascade/parallel formant synthesizer](https://doi.org/10.1121/1.383940),
//!   Table I: typical values for the parameters of a formant synthesiser.
//! - Rosenberg (1971), Effect of glottal pulse shape on the quality of natural vowels,
//!   Journal of the Acoustical Society of America 49, 583–590: the shape of the glottal pulse.
//!
//! Hand-picked values only need to sound plausible.
//! The ground truth is whatever the bank was rendered from, which [`TRUTH_FILE_NAME`] records.

use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::io;
use std::path::Path;

use fs_err as fs;
use rand::{RngExt, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::filter::{Antiresonator, Cascade, Formant, Resonator};
use crate::oto::{Cutoff, OtoEntry};
use crate::quantity::{Fraction, Frequency, Gain, Interval, SampleRate, Time};
use crate::voicebank::{Voicebank, VoicebankError, VoicebankSample};

/// The file, next to the bank's oto.ini, that records what the bank was rendered from.
pub const TRUTH_FILE_NAME: &str = "synthetic-truth.toml";

/// Noise comes from a named generator instead of `rand::rngs::StdRng`,
/// whose output may change between versions of rand.
/// See [the rand book on reproducibility](https://rust-random.github.io/book/crate-reprod.html).
type Random = Xoshiro256PlusPlus;

/// F1 to F3 of the men aged around 20 in Nakagawa et al. (1980), Table 1, group M2, in the order a, i, u, e, o.
const NAKAGAWA_MEN: [(&str, [Frequency; 3]); 5] = [
    (
        "a",
        [
            Frequency::from_hertz(832.0),
            Frequency::from_hertz(1234.0),
            Frequency::from_hertz(2668.0),
        ],
    ),
    (
        "i",
        [
            Frequency::from_hertz(338.0),
            Frequency::from_hertz(2207.0),
            Frequency::from_hertz(3118.0),
        ],
    ),
    (
        "u",
        [
            Frequency::from_hertz(396.0),
            Frequency::from_hertz(1215.0),
            Frequency::from_hertz(2302.0),
        ],
    ),
    (
        "e",
        [
            Frequency::from_hertz(578.0),
            Frequency::from_hertz(1894.0),
            Frequency::from_hertz(2675.0),
        ],
    ),
    (
        "o",
        [
            Frequency::from_hertz(593.0),
            Frequency::from_hertz(894.0),
            Frequency::from_hertz(2730.0),
        ],
    ),
];

/// The typical F4 in Klatt (1980), Table I.
/// Nakagawa et al. do not measure F4.
const KLATT_F4: Frequency = Frequency::from_hertz(3300.0);

/// The typical B1 to B4 in Klatt (1980), Table I.
/// Klatt's values are for English, and Nakagawa et al. do not measure bandwidths.
const KLATT_BANDWIDTHS: [Frequency; 4] = [
    Frequency::from_hertz(50.0),
    Frequency::from_hertz(70.0),
    Frequency::from_hertz(110.0),
    Frequency::from_hertz(250.0),
];

/// F1 to F4 of one vowel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VowelFormants {
    pub formants: [Formant; 4],
}

/// The five Japanese vowels of a young adult man.
#[must_use]
pub fn japanese_vowels() -> Vec<(&'static str, VowelFormants)> {
    NAKAGAWA_MEN
        .iter()
        .map(|&(id, [first, second, third])| {
            let frequencies = [first, second, third, KLATT_F4];

            let formants = std::array::from_fn(|index| Formant {
                frequency: frequencies[index],
                bandwidth: KLATT_BANDWIDTHS[index],
            });

            (id, VowelFormants { formants })
        })
        .collect()
}

/// A consonant made of filtered noise, such as a fricative or the burst of a stop.
#[derive(Debug, Clone, Copy)]
struct NoiseConsonant {
    /// Silence before the noise, as in the closure of a stop.
    closure: Time,
    duration: Time,
    /// The single resonance that shapes the noise.
    shape: Formant,
    /// Peak level, as a fraction of full scale.
    level: Fraction,
}

/// A nasal consonant: the glottal source through a low resonance and an antiresonance.
#[derive(Debug, Clone, Copy)]
struct NasalConsonant {
    duration: Time,
    /// The notch from the closed mouth, which is lower for m than for n.
    antiresonance: Formant,
}

#[derive(Debug, Clone, Copy)]
enum ConsonantSound {
    Noise(NoiseConsonant),
    Nasal(NasalConsonant),
}

impl ConsonantSound {
    /// Length including any closure.
    fn duration(self) -> Time {
        match self {
            ConsonantSound::Noise(noise) => noise.closure + noise.duration,
            ConsonantSound::Nasal(nasal) => nasal.duration,
        }
    }
}

/// The nasal murmur's resonance: the typical FNP and BNP in Klatt (1980), Table I.
const NASAL_MURMUR: Formant = Formant {
    frequency: Frequency::from_hertz(250.0),
    bandwidth: Frequency::from_hertz(100.0),
};

/// The typical BNZ in Klatt (1980), Table I.
const NASAL_ANTIRESONANCE_BANDWIDTH: Frequency = Frequency::from_hertz(100.0);

/// Peak level of nasal murmur, as a fraction of full scale.
/// Hand-picked to sit below the vowel's [`VOWEL_LEVEL`].
const NASAL_LEVEL: Fraction = Fraction::new(0.2);

// Consonants, all hand-picked.
// Stops get a closure, and sibilants get high noise, s higher than sh, as fricatives show
// in [Jongman, Wayland and Wong (2000)](https://doi.org/10.1121/1.1288413).

/// Silence before the burst of a stop.
const STOP_CLOSURE: Time = Time::from_seconds(0.04);

const NASAL_DURATION: Time = Time::from_seconds(0.07);

/// A short burst with energy in the middle of the spectrum.
const CONSONANT_K: ConsonantSound = ConsonantSound::Noise(NoiseConsonant {
    closure: STOP_CLOSURE,
    duration: Time::from_seconds(0.03),
    shape: Formant {
        frequency: Frequency::from_hertz(2000.0),
        bandwidth: Frequency::from_hertz(1200.0),
    },
    level: Fraction::new(0.3),
});

/// Long, high-pitched hiss.
const CONSONANT_S: ConsonantSound = ConsonantSound::Noise(NoiseConsonant {
    closure: Time::ZERO,
    duration: Time::from_seconds(0.12),
    shape: Formant {
        frequency: Frequency::from_hertz(6000.0),
        bandwidth: Frequency::from_hertz(2500.0),
    },
    level: Fraction::new(0.2),
});

/// Long hiss, lower than s.
const CONSONANT_SH: ConsonantSound = ConsonantSound::Noise(NoiseConsonant {
    closure: Time::ZERO,
    duration: Time::from_seconds(0.12),
    shape: Formant {
        frequency: Frequency::from_hertz(3500.0),
        bandwidth: Frequency::from_hertz(1500.0),
    },
    level: Fraction::new(0.25),
});

/// A short, high burst.
const CONSONANT_T: ConsonantSound = ConsonantSound::Noise(NoiseConsonant {
    closure: STOP_CLOSURE,
    duration: Time::from_seconds(0.02),
    shape: Formant {
        frequency: Frequency::from_hertz(4000.0),
        bandwidth: Frequency::from_hertz(2000.0),
    },
    level: Fraction::new(0.3),
});

/// A closure released into sh-like noise.
const CONSONANT_CH: ConsonantSound = ConsonantSound::Noise(NoiseConsonant {
    closure: STOP_CLOSURE,
    duration: Time::from_seconds(0.08),
    shape: Formant {
        frequency: Frequency::from_hertz(3500.0),
        bandwidth: Frequency::from_hertz(1500.0),
    },
    level: Fraction::new(0.25),
});

/// A closure released into s-like noise.
const CONSONANT_TS: ConsonantSound = ConsonantSound::Noise(NoiseConsonant {
    closure: STOP_CLOSURE,
    duration: Time::from_seconds(0.08),
    shape: Formant {
        frequency: Frequency::from_hertz(6000.0),
        bandwidth: Frequency::from_hertz(2500.0),
    },
    level: Fraction::new(0.2),
});

/// Quiet, broad breath noise.
const CONSONANT_H: ConsonantSound = ConsonantSound::Noise(NoiseConsonant {
    closure: Time::ZERO,
    duration: Time::from_seconds(0.08),
    shape: Formant {
        frequency: Frequency::from_hertz(1500.0),
        bandwidth: Frequency::from_hertz(2500.0),
    },
    level: Fraction::new(0.1),
});

/// Japanese f, a quiet bilabial breath, lower than h.
const CONSONANT_F: ConsonantSound = ConsonantSound::Noise(NoiseConsonant {
    closure: Time::ZERO,
    duration: Time::from_seconds(0.08),
    shape: Formant {
        frequency: Frequency::from_hertz(1200.0),
        bandwidth: Frequency::from_hertz(2500.0),
    },
    level: Fraction::new(0.08),
});

/// The antiresonance of the alveolar nasal.
const CONSONANT_N: ConsonantSound = ConsonantSound::Nasal(NasalConsonant {
    duration: NASAL_DURATION,
    antiresonance: Formant {
        frequency: Frequency::from_hertz(1800.0),
        bandwidth: NASAL_ANTIRESONANCE_BANDWIDTH,
    },
});

/// The antiresonance of the bilabial nasal, lower than n's.
const CONSONANT_M: ConsonantSound = ConsonantSound::Nasal(NasalConsonant {
    duration: NASAL_DURATION,
    antiresonance: Formant {
        frequency: Frequency::from_hertz(1000.0),
        bandwidth: NASAL_ANTIRESONANCE_BANDWIDTH,
    },
});

fn consonant_sound(consonant: &str) -> Option<ConsonantSound> {
    let sound = match consonant {
        "k" => CONSONANT_K,
        "s" => CONSONANT_S,
        "sh" => CONSONANT_SH,
        "t" => CONSONANT_T,
        "ch" => CONSONANT_CH,
        "ts" => CONSONANT_TS,
        "h" => CONSONANT_H,
        "f" => CONSONANT_F,
        "n" => CONSONANT_N,
        "m" => CONSONANT_M,
        _ => return None,
    };

    Some(sound)
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

/// The rows of the kana table the bank covers; `""` is the row of bare vowels.
const CONSONANT_ROWS: [&str; 7] = ["", "k", "s", "t", "n", "h", "m"];

#[derive(Debug, Clone)]
pub struct SyntheticBankSettings {
    pub name: String,
    pub sample_rate: SampleRate,
    /// The centre of the pitch band the samples spread across.
    pub base_fundamental_frequency: Frequency,
    /// Width of the pitch band.
    pub pitch_spread: Interval,
    pub seed: u64,
}

impl Default for SyntheticBankSettings {
    fn default() -> Self {
        Self {
            name: "Synthetic CV".to_string(),
            sample_rate: SampleRate::from_hertz(44_100),
            // Near the mean pitch of 146 Hz in Nakagawa et al. (1980), Table 1, group M2, whose formants the bank uses.
            base_fundamental_frequency: Frequency::from_hertz(146.0),
            pitch_spread: Interval::from_semitones(4.0),
            seed: 1,
        }
    }
}

/// What one sample was rendered from.
/// Times count from the start of the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleTruth {
    pub file_name: String,
    pub alias: String,
    pub consonant: String,
    pub vowel: String,
    pub fundamental_frequency: Frequency,
    pub consonant_start: Time,
    pub vowel_start: Time,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyntheticTruth {
    pub sample_rate: SampleRate,
    pub base_fundamental_frequency: Frequency,
    pub vowels: BTreeMap<String, VowelFormants>,
    pub samples: Vec<SampleTruth>,
}

#[derive(Debug, Error)]
pub enum SyntheticError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("could not write the voicebank")]
    Voicebank(#[from] VoicebankError),
    #[error("could not encode the truth file")]
    Serialize(#[from] toml::ser::Error),
    #[error("could not parse the truth file")]
    Deserialize(#[from] toml::de::Error),
}

impl SyntheticTruth {
    /// Reads a truth file written by [`SyntheticBank::write`].
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or is not a valid truth file.
    pub fn read<P>(path: P) -> Result<Self, SyntheticError>
    where
        P: AsRef<Path>,
    {
        let text = fs::read_to_string(path.as_ref())?;

        let truth = toml::from_str(&text)?;

        Ok(truth)
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
    pub fn write<P>(&self, directory: P) -> Result<(), SyntheticError>
    where
        P: AsRef<Path>,
    {
        let directory = directory.as_ref();

        self.voicebank.write(directory)?;

        let truth = toml::to_string(&self.truth)?;

        fs::write(directory.join(TRUTH_FILE_NAME), truth)?;

        Ok(())
    }
}

/// White noise from −1 to 1.
fn white_noise(random: &mut Random) -> f64 {
    random.random_range(-1.0..1.0)
}

/// Glottal flow derivative from Rosenberg pulses, with slight period jitter.
struct GlottalSource {
    sample_rate: SampleRate,
    fundamental_frequency: Frequency,
    /// The fundamental frequency of the current period, after jitter.
    period_frequency: Frequency,
    /// Position in the current period.
    phase: Fraction,
    previous_flow: f64,
}

impl GlottalSource {
    /// Share of each period the vocal folds spend opening, then closing; Rosenberg's TP and TN.
    /// Hand-picked; these are common defaults for Rosenberg's trigonometric pulse.
    const OPENING: Fraction = Fraction::new(0.4);
    const CLOSING: Fraction = Fraction::new(0.16);

    /// Largest change of one period's length, relative to the period.
    /// Hand-picked and small, to make the voice less mechanical without making it rough.
    const JITTER: Fraction = Fraction::new(0.005);

    fn new(fundamental_frequency: Frequency, sample_rate: SampleRate) -> Self {
        Self {
            sample_rate,
            fundamental_frequency,
            period_frequency: fundamental_frequency,
            phase: Fraction::ZERO,
            previous_flow: 0.0,
        }
    }

    /// Glottal flow from 0 to 1 at a phase of the period.
    fn flow(phase: Fraction) -> f64 {
        let phase = phase.value();
        let opening = Self::OPENING.value();
        let closing = Self::CLOSING.value();

        if phase < opening {
            let opening_progress = phase / opening;

            0.5 * (1.0 - (PI * opening_progress).cos())
        } else if phase < opening + closing {
            let closing_progress = (phase - opening) / closing;

            (0.5 * PI * closing_progress).cos()
        } else {
            0.0
        }
    }

    fn next_sample(&mut self, random: &mut Random) -> f64 {
        let flow = Self::flow(self.phase);

        // Differentiating the flow models radiation from the lips.
        let derivative = flow - self.previous_flow;

        self.previous_flow = flow;

        let phase_step = self.period_frequency / self.sample_rate.frequency();
        let next_phase = self.phase.value() + phase_step;

        self.phase = Fraction::new(next_phase.fract());

        let period_ended = next_phase >= 1.0;

        if period_ended {
            let jitter = Self::JITTER.value() * white_noise(random);

            self.period_frequency = self.fundamental_frequency * (1.0 + jitter);
        }

        derivative
    }
}

// Timing of each sample, all hand-picked.

/// Silence before the consonant, so the oto offset has room.
const LEADING_SILENCE: Time = Time::from_seconds(0.1);
/// Silence after the vowel.
const TRAILING_SILENCE: Time = Time::from_seconds(0.1);
/// Length of the vowel including its attack and release, long enough to stretch over most notes.
const VOWEL_DURATION: Time = Time::from_seconds(0.8);
/// Fade-in of a bare vowel.
const VOWEL_ATTACK: Time = Time::from_seconds(0.03);
/// Fade-out of every vowel.
const VOWEL_RELEASE: Time = Time::from_seconds(0.08);
/// Overlap where a consonant fades out and its vowel fades in.
const CROSSFADE: Time = Time::from_seconds(0.015);
/// Fade-in of a noise consonant, short like the release of a stop.
const NOISE_ATTACK: Time = Time::from_seconds(0.005);
/// Fade-in of a nasal murmur.
const NASAL_ATTACK: Time = Time::from_seconds(0.01);
/// Peak level of the vowel, as a fraction of full scale, leaving headroom for the consonant crossfade.
const VOWEL_LEVEL: Fraction = Fraction::new(0.5);

// The oto.ini timing of each sample, all hand-picked.

/// How long before the consonant the sample starts.
const OTO_LEAD_IN: Time = Time::from_milliseconds(30.0);
/// How far into the vowel the fixed region reaches, past the transition.
const OTO_FIXED_VOWEL: Time = Time::from_milliseconds(50.0);
/// Crossfade with the previous note for a sample with a consonant.
const OTO_CONSONANT_OVERLAP: Time = Time::from_milliseconds(20.0);
/// Crossfade with the previous note for a bare vowel.
const OTO_VOWEL_OVERLAP: Time = Time::from_milliseconds(10.0);

/// Timing of one rendered sample, from the start of the file.
struct Layout {
    consonant_start: Time,
    vowel_start: Time,
    vowel_end: Time,
    length: Time,
}

impl Layout {
    fn new(consonant: Option<ConsonantSound>) -> Self {
        let consonant_duration = consonant.map_or(Time::ZERO, ConsonantSound::duration);

        let consonant_start = LEADING_SILENCE;
        let vowel_start = consonant_start + consonant_duration;
        let vowel_end = vowel_start + VOWEL_DURATION;
        let length = vowel_end + TRAILING_SILENCE;

        Self {
            consonant_start,
            vowel_start,
            vowel_end,
            length,
        }
    }

    fn oto_entry(&self, file_name: String, alias: String, has_consonant: bool) -> OtoEntry {
        // Whole milliseconds keep the oto.ini readable.
        let round = |time: Time| Time::from_milliseconds(time.milliseconds().round());

        let offset = round(self.consonant_start) - OTO_LEAD_IN;
        let preutterance = round(self.vowel_start) - offset;
        let consonant = preutterance + OTO_FIXED_VOWEL;
        let sample_length = round(self.vowel_end - VOWEL_RELEASE) - offset;

        let overlap = if has_consonant {
            OTO_CONSONANT_OVERLAP
        } else {
            OTO_VOWEL_OVERLAP
        };

        OtoEntry {
            file_name,
            alias,
            offset,
            consonant,
            cutoff: Cutoff::AfterOffset(sample_length),
            preutterance,
            overlap,
        }
    }
}

/// A gain from 0 to 1 that rises linearly over `attack` from `start`, and falls over `release` until `end`.
struct Envelope {
    start: Time,
    attack: Time,
    end: Time,
    release: Time,
}

impl Envelope {
    fn gain(&self, time: Time) -> Gain {
        if time < self.start || time >= self.end {
            return Gain::SILENCE;
        }

        let rising = (time - self.start) / self.attack;
        let falling = (self.end - time) / self.release;

        let linear = rising.min(falling).min(1.0);

        Gain::from_linear(linear)
    }
}

/// Scales a signal so its peak is at `level`.
fn normalize(signal: &mut [f64], level: Fraction) {
    let peak = signal
        .iter()
        .copied()
        .map(f64::abs)
        .reduce(f64::max)
        .unwrap_or(0.0);

    if peak > 0.0 {
        for sample in signal {
            *sample *= level.value() / peak;
        }
    }
}

fn render(
    vowel: &VowelFormants,
    consonant: Option<ConsonantSound>,
    layout: &Layout,
    fundamental_frequency: Frequency,
    sample_rate: SampleRate,
    random: &mut Random,
) -> Vec<f64> {
    let sample_count = sample_rate.sample_count(layout.length);
    let times: Vec<Time> = (0..sample_count)
        .map(|index| sample_rate.time_of_sample(index))
        .collect();

    let mut source = GlottalSource::new(fundamental_frequency, sample_rate);
    let glottal: Vec<_> = times.iter().map(|_| source.next_sample(random)).collect();

    // A consonant hands over to the vowel with a crossfade, a bare vowel starts with a plain attack.
    let vowel_attack = if consonant.is_some() {
        CROSSFADE
    } else {
        VOWEL_ATTACK
    };

    let vowel_envelope = Envelope {
        start: layout.vowel_start,
        attack: vowel_attack,
        end: layout.vowel_end,
        release: VOWEL_RELEASE,
    };

    let mut tract = Cascade::new(&vowel.formants, sample_rate);

    let mut output: Vec<f64> = glottal
        .iter()
        .zip(&times)
        .map(|(&sample, &time)| tract.process_sample(sample) * vowel_envelope.gain(time))
        .collect();

    normalize(&mut output, VOWEL_LEVEL);

    let consonant_end = layout.vowel_start + CROSSFADE;

    let consonant_part: Vec<f64> = match consonant {
        None => Vec::new(),
        Some(ConsonantSound::Noise(noise)) => {
            let envelope = Envelope {
                start: layout.consonant_start + noise.closure,
                attack: NOISE_ATTACK,
                end: consonant_end,
                release: CROSSFADE,
            };

            let mut shaping = Resonator::new(noise.shape, sample_rate);

            let mut part: Vec<f64> = times
                .iter()
                .map(|&time| shaping.process_sample(white_noise(random)) * envelope.gain(time))
                .collect();

            normalize(&mut part, noise.level);

            part
        }
        Some(ConsonantSound::Nasal(nasal)) => {
            let envelope = Envelope {
                start: layout.consonant_start,
                attack: NASAL_ATTACK,
                end: consonant_end,
                release: CROSSFADE,
            };

            let mut murmur = Resonator::new(NASAL_MURMUR, sample_rate);
            let mut antiresonance = Antiresonator::new(nasal.antiresonance, sample_rate);

            let mut part: Vec<f64> = glottal
                .iter()
                .zip(&times)
                .map(|(&sample, &time)| {
                    let murmured = murmur.process_sample(sample);

                    antiresonance.process_sample(murmured) * envelope.gain(time)
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
/// Successive multiples of the golden ratio, modulo 1, fill an interval more evenly than random numbers
/// (a [Weyl sequence](https://en.wikipedia.org/wiki/Low-discrepancy_sequence#Additive_recurrence)).
fn fundamental_frequency(settings: &SyntheticBankSettings, sample_index: usize) -> Frequency {
    const GOLDEN_RATIO_CONJUGATE: f64 = 0.618_033_988_749_895;

    #[expect(
        clippy::cast_precision_loss,
        reason = "sample indices stay far below 2^52"
    )]
    let sample_index = sample_index as f64;

    let position_in_band = Fraction::new((sample_index * GOLDEN_RATIO_CONJUGATE).fract());

    // Centres the band on the base frequency.
    let offset_from_centre = position_in_band.value() - 0.5;
    let interval = settings.pitch_spread * offset_from_centre;

    settings.base_fundamental_frequency * interval
}

#[must_use]
pub fn generate(settings: &SyntheticBankSettings) -> SyntheticBank {
    let vowels = japanese_vowels();
    let mut random = Random::seed_from_u64(settings.seed);
    let mut samples = Vec::new();
    let mut sample_truths = Vec::new();

    for row in CONSONANT_ROWS {
        for (vowel_id, vowel) in &vowels {
            let consonant_id = japanese_consonant(row, vowel_id);
            let consonant = consonant_sound(consonant_id);

            let alias = format!("{consonant_id}{vowel_id}");
            let file_name = format!("{alias}.wav");

            let fundamental_frequency = fundamental_frequency(settings, samples.len());
            let layout = Layout::new(consonant);

            let audio = render(
                vowel,
                consonant,
                &layout,
                fundamental_frequency,
                settings.sample_rate,
                &mut random,
            );

            let entry = layout.oto_entry(file_name.clone(), alias.clone(), consonant.is_some());

            samples.push(VoicebankSample { audio, entry });

            sample_truths.push(SampleTruth {
                file_name,
                alias,
                consonant: consonant_id.to_string(),
                vowel: vowel_id.to_string(),
                fundamental_frequency,
                consonant_start: layout.consonant_start,
                vowel_start: layout.vowel_start,
            });
        }
    }

    let voicebank = Voicebank {
        name: settings.name.clone(),
        sample_rate: settings.sample_rate,
        samples,
    };

    let truth = SyntheticTruth {
        sample_rate: settings.sample_rate,
        base_fundamental_frequency: settings.base_fundamental_frequency,
        vowels: vowels
            .into_iter()
            .map(|(id, formants)| (id.to_string(), formants))
            .collect(),
        samples: sample_truths,
    };

    SyntheticBank { voicebank, truth }
}
