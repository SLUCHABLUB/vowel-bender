//! Formant filters from [Klatt (1980)](https://doi.org/10.1121/1.383940), equations 1 to 5.
//!
//! A resonator is a two-pole low-pass with a resonant peak at the formant frequency and unity gain at DC.
//! An antiresonator is its exact inverse, so a resonator followed by an antiresonator with the same settings is flat.
//!
//! Names here describe what a value does.
//! Comments give the symbol Klatt uses for it, so the code can be read next to the paper.

use std::f64::consts::PI;

use num_complex::Complex64;
use serde::{Deserialize, Serialize};

use crate::quantity::{Frequency, Gain, SampleRate};

/// A resonance of the vocal tract: a peak at `frequency` whose −3 dB width is `bandwidth`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Formant {
    /// Klatt's F.
    pub frequency: Frequency,
    /// Klatt's BW.
    pub bandwidth: Frequency,
}

/// The weights of Klatt's equation 1, shared by a resonator and its antiresonator.
///
/// A resonator computes `output = input * input_sample + previous * previous_output + second_previous * second_previous_output`.
/// An antiresonator solves the same equation for the input, so its weights apply to previous inputs.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Weights {
    /// Weight of the current input; Klatt's A.
    input: f64,
    /// Weight of the previous sample; Klatt's B.
    previous: f64,
    /// Weight of the sample before that; Klatt's C.
    second_previous: f64,
}

impl Weights {
    /// Klatt's equations 2 and 3, an impulse-invariant transform of an analog resonance.
    fn new(formant: Formant, sample_rate: SampleRate) -> Self {
        let sample_rate = sample_rate.frequency();

        // The resonance is a pair of poles at this distance from the origin and these angles above and below the real axis.
        // Klatt writes the radius as exp(-π BW T) and the angle as 2π F T, where T is the sample period.
        let pole_radius = (-PI * (formant.bandwidth / sample_rate)).exp();
        let pole_angle = 2.0 * PI * (formant.frequency / sample_rate);

        let second_previous_weight = -pole_radius * pole_radius;
        let previous_weight = 2.0 * pole_radius * pole_angle.cos();

        // Klatt's equation 3, which makes the gain at DC exactly 1.
        let input_weight = 1.0 - previous_weight - second_previous_weight;

        Self {
            input: input_weight,
            previous: previous_weight,
            second_previous: second_previous_weight,
        }
    }

    /// The resonator's transfer function at a frequency,
    /// `input / (1 - previous * delay - second_previous * delay^2)`,
    /// where `delay` is one sample of delay at that frequency (Klatt's z^-1).
    fn resonator_transfer(&self, frequency: Frequency, sample_rate: SampleRate) -> Complex64 {
        let angular_frequency = 2.0 * PI * (frequency / sample_rate.frequency());

        let delay = Complex64::from_polar(1.0, -angular_frequency);

        let denominator = 1.0 - self.previous * delay - self.second_previous * delay * delay;

        self.input / denominator
    }
}

/// A filter for one formant.
#[derive(Debug, Clone)]
pub struct Resonator {
    sample_rate: SampleRate,
    formant: Formant,
    weights: Weights,
    /// Klatt's y(nT - T) and y(nT - 2T).
    previous_outputs: [f64; 2],
}

impl Resonator {
    #[must_use]
    pub fn new(formant: Formant, sample_rate: SampleRate) -> Self {
        Self {
            sample_rate,
            formant,
            weights: Weights::new(formant, sample_rate),
            previous_outputs: [0.0; 2],
        }
    }

    #[must_use]
    pub fn formant(&self) -> Formant {
        self.formant
    }

    /// Changes the formant while keeping the filter's state, so the peak can glide without clicks.
    pub fn set_formant(&mut self, formant: Formant) {
        self.formant = formant;
        self.weights = Weights::new(formant, self.sample_rate);
    }

    pub fn reset(&mut self) {
        self.previous_outputs = [0.0; 2];
    }

    /// Klatt's equation 1.
    pub fn process_sample(&mut self, input: f64) -> f64 {
        let Weights {
            input: input_weight,
            previous: previous_weight,
            second_previous: second_previous_weight,
        } = self.weights;

        let [previous_output, second_previous_output] = self.previous_outputs;

        let output = input_weight * input
            + previous_weight * previous_output
            + second_previous_weight * second_previous_output;

        self.previous_outputs = [output, previous_output];

        output
    }

    pub fn process_buffer(&mut self, buffer: &mut [f64]) {
        for sample in buffer {
            *sample = self.process_sample(*sample);
        }
    }

    #[must_use]
    pub fn frequency_response(&self, frequency: Frequency) -> Gain {
        let transfer = self.weights.resonator_transfer(frequency, self.sample_rate);

        Gain::from_linear(transfer.norm())
    }
}

/// The exact inverse of a [`Resonator`] with the same settings: a notch that undoes a formant.
#[derive(Debug, Clone)]
pub struct Antiresonator {
    sample_rate: SampleRate,
    formant: Formant,
    weights: Weights,
    /// Klatt's x(nT - T) and x(nT - 2T).
    previous_inputs: [f64; 2],
}

impl Antiresonator {
    #[must_use]
    pub fn new(formant: Formant, sample_rate: SampleRate) -> Self {
        Self {
            sample_rate,
            formant,
            weights: Weights::new(formant, sample_rate),
            previous_inputs: [0.0; 2],
        }
    }

    #[must_use]
    pub fn formant(&self) -> Formant {
        self.formant
    }

    /// Changes the notch while keeping the filter's state.
    pub fn set_formant(&mut self, formant: Formant) {
        self.formant = formant;
        self.weights = Weights::new(formant, self.sample_rate);
    }

    pub fn reset(&mut self) {
        self.previous_inputs = [0.0; 2];
    }

    /// Klatt's equations 4 and 5: equation 1 solved for the input,
    /// with weights A' = 1 / A, B' = -B / A and C' = -C / A.
    pub fn process_sample(&mut self, input: f64) -> f64 {
        let Weights {
            input: input_weight,
            previous: previous_weight,
            second_previous: second_previous_weight,
        } = self.weights;

        let [previous_input, second_previous_input] = self.previous_inputs;

        let output = (input
            - previous_weight * previous_input
            - second_previous_weight * second_previous_input)
            / input_weight;

        self.previous_inputs = [input, previous_input];

        output
    }

    pub fn process_buffer(&mut self, buffer: &mut [f64]) {
        for sample in buffer {
            *sample = self.process_sample(*sample);
        }
    }

    #[must_use]
    pub fn frequency_response(&self, frequency: Frequency) -> Gain {
        let transfer = self
            .weights
            .resonator_transfer(frequency, self.sample_rate)
            .inv();

        Gain::from_linear(transfer.norm())
    }
}

/// Resonators applied in series, as in a vocal tract model.
#[derive(Debug, Clone, Default)]
pub struct Cascade {
    resonators: Vec<Resonator>,
}

impl Cascade {
    #[must_use]
    pub fn new(formants: &[Formant], sample_rate: SampleRate) -> Self {
        let resonators = formants
            .iter()
            .map(|&formant| Resonator::new(formant, sample_rate))
            .collect();

        Self { resonators }
    }

    #[must_use]
    pub fn resonators(&self) -> &[Resonator] {
        &self.resonators
    }

    pub fn reset(&mut self) {
        for resonator in &mut self.resonators {
            resonator.reset();
        }
    }

    pub fn process_sample(&mut self, input: f64) -> f64 {
        self.resonators
            .iter_mut()
            .fold(input, |signal, resonator| resonator.process_sample(signal))
    }

    pub fn process_buffer(&mut self, buffer: &mut [f64]) {
        for sample in buffer {
            *sample = self.process_sample(*sample);
        }
    }

    #[must_use]
    pub fn frequency_response(&self, frequency: Frequency) -> Gain {
        self.resonators
            .iter()
            .map(|resonator| resonator.frequency_response(frequency))
            .product()
    }
}
