//! Formant filters after Klatt (1980).
//!
//! A resonator is a two-pole low-pass with a resonant peak at the formant frequency and unity gain at DC.
//! An antiresonator is its exact inverse, so a resonator followed by an antiresonator with the same settings is flat.

use std::f64::consts::PI;

/// Coefficients shared by a resonator and its antiresonator.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Coefficients {
    a: f64,
    b: f64,
    c: f64,
}

impl Coefficients {
    fn new(frequency: f64, bandwidth: f64, sample_rate: f64) -> Self {
        let c = -(-2.0 * PI * bandwidth / sample_rate).exp();
        let b = 2.0
            * (-PI * bandwidth / sample_rate).exp()
            * (2.0 * PI * frequency / sample_rate).cos();
        let a = 1.0 - b - c;
        Self { a, b, c }
    }

    /// Magnitude of the resonator's response at a frequency.
    /// The resonator's transfer function is a / (1 - b·z⁻¹ - c·z⁻²).
    fn resonator_magnitude(&self, frequency: f64, sample_rate: f64) -> f64 {
        let omega = 2.0 * PI * frequency / sample_rate;
        let real = 1.0 - self.b * omega.cos() - self.c * (2.0 * omega).cos();
        let imaginary = self.b * omega.sin() + self.c * (2.0 * omega).sin();
        self.a / real.hypot(imaginary)
    }
}

/// One formant: a resonant peak at `frequency` whose −3 dB width is `bandwidth`, both in Hz.
#[derive(Debug, Clone)]
pub struct Resonator {
    sample_rate: f64,
    frequency: f64,
    bandwidth: f64,
    coefficients: Coefficients,
    previous_outputs: [f64; 2],
}

impl Resonator {
    pub fn new(frequency: f64, bandwidth: f64, sample_rate: f64) -> Self {
        Self {
            sample_rate,
            frequency,
            bandwidth,
            coefficients: Coefficients::new(frequency, bandwidth, sample_rate),
            previous_outputs: [0.0; 2],
        }
    }

    pub fn frequency(&self) -> f64 {
        self.frequency
    }

    pub fn bandwidth(&self) -> f64 {
        self.bandwidth
    }

    /// Changes the formant while keeping the filter's state, so the peak can glide without clicks.
    pub fn set_parameters(&mut self, frequency: f64, bandwidth: f64) {
        self.frequency = frequency;
        self.bandwidth = bandwidth;
        self.coefficients = Coefficients::new(frequency, bandwidth, self.sample_rate);
    }

    pub fn reset(&mut self) {
        self.previous_outputs = [0.0; 2];
    }

    pub fn process_sample(&mut self, input: f64) -> f64 {
        let Coefficients { a, b, c } = self.coefficients;
        let output = a * input + b * self.previous_outputs[0] + c * self.previous_outputs[1];
        self.previous_outputs = [output, self.previous_outputs[0]];
        output
    }

    pub fn process_buffer(&mut self, buffer: &mut [f64]) {
        for sample in buffer {
            *sample = self.process_sample(*sample);
        }
    }

    /// Linear gain at a frequency in Hz.
    pub fn frequency_response(&self, frequency: f64) -> f64 {
        self.coefficients
            .resonator_magnitude(frequency, self.sample_rate)
    }
}

/// The exact inverse of a [`Resonator`] with the same settings: a notch that undoes a formant.
#[derive(Debug, Clone)]
pub struct Antiresonator {
    sample_rate: f64,
    frequency: f64,
    bandwidth: f64,
    coefficients: Coefficients,
    previous_inputs: [f64; 2],
}

impl Antiresonator {
    pub fn new(frequency: f64, bandwidth: f64, sample_rate: f64) -> Self {
        Self {
            sample_rate,
            frequency,
            bandwidth,
            coefficients: Coefficients::new(frequency, bandwidth, sample_rate),
            previous_inputs: [0.0; 2],
        }
    }

    pub fn frequency(&self) -> f64 {
        self.frequency
    }

    pub fn bandwidth(&self) -> f64 {
        self.bandwidth
    }

    /// Changes the notch while keeping the filter's state.
    pub fn set_parameters(&mut self, frequency: f64, bandwidth: f64) {
        self.frequency = frequency;
        self.bandwidth = bandwidth;
        self.coefficients = Coefficients::new(frequency, bandwidth, self.sample_rate);
    }

    pub fn reset(&mut self) {
        self.previous_inputs = [0.0; 2];
    }

    pub fn process_sample(&mut self, input: f64) -> f64 {
        let Coefficients { a, b, c } = self.coefficients;
        let output = (input - b * self.previous_inputs[0] - c * self.previous_inputs[1]) / a;
        self.previous_inputs = [input, self.previous_inputs[0]];
        output
    }

    pub fn process_buffer(&mut self, buffer: &mut [f64]) {
        for sample in buffer {
            *sample = self.process_sample(*sample);
        }
    }

    /// Linear gain at a frequency in Hz.
    pub fn frequency_response(&self, frequency: f64) -> f64 {
        1.0 / self
            .coefficients
            .resonator_magnitude(frequency, self.sample_rate)
    }
}

/// Resonators applied in series, as in a vocal tract model.
#[derive(Debug, Clone, Default)]
pub struct Cascade {
    resonators: Vec<Resonator>,
}

impl Cascade {
    /// Builds a cascade from `(frequency, bandwidth)` pairs.
    pub fn new(formants: &[(f64, f64)], sample_rate: f64) -> Self {
        let resonators = formants
            .iter()
            .map(|&(frequency, bandwidth)| Resonator::new(frequency, bandwidth, sample_rate))
            .collect();
        Self { resonators }
    }

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

    /// Linear gain at a frequency in Hz.
    pub fn frequency_response(&self, frequency: f64) -> f64 {
        self.resonators
            .iter()
            .map(|resonator| resonator.frequency_response(frequency))
            .product()
    }
}
