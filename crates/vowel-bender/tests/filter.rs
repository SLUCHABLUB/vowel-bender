use vowel_bender::filter::{Antiresonator, Cascade, Resonator};
use vowel_bender::synthetic::NoiseGenerator;

const SAMPLE_RATE: f64 = 44_100.0;

/// Formants with bandwidth well below frequency, and frequency well below Nyquist,
/// where a digital resonator's −3 dB width closely matches its bandwidth setting.
const FORMANTS: [(f64, f64); 5] = [
    (500.0, 60.0),
    (1000.0, 100.0),
    (1500.0, 90.0),
    (2500.0, 150.0),
    (3500.0, 200.0),
];

/// Step of the frequency scans, in Hz.
const SCAN_STEP: f64 = 0.1;

fn noise(length: usize) -> Vec<f64> {
    let mut generator = NoiseGenerator::new(7);
    (0..length).map(|_| generator.next_sample()).collect()
}

#[test]
fn antiresonator_undoes_resonator() {
    let input = noise(44_100);
    for (frequency, bandwidth) in FORMANTS {
        let mut signal = input.clone();
        Resonator::new(frequency, bandwidth, SAMPLE_RATE).process_buffer(&mut signal);
        Antiresonator::new(frequency, bandwidth, SAMPLE_RATE).process_buffer(&mut signal);
        let largest_error = input
            .iter()
            .zip(&signal)
            .fold(0.0_f64, |largest, (original, restored)| {
                largest.max((original - restored).abs())
            });
        assert!(
            largest_error < 1e-9,
            "{frequency} Hz: error {largest_error}"
        );
    }
}

#[test]
fn resonator_peaks_at_its_frequency() {
    for (frequency, bandwidth) in FORMANTS {
        let resonator = Resonator::new(frequency, bandwidth, SAMPLE_RATE);
        let peak = peak_frequency(&resonator);
        assert!(
            (peak - frequency).abs() / frequency < 0.01,
            "set {frequency} Hz, peak at {peak} Hz"
        );
    }
}

#[test]
fn resonator_width_matches_bandwidth() {
    // The match is approximate, closest for narrow bandwidths far below Nyquist.
    const TOLERANCE: f64 = 0.05;
    for (frequency, bandwidth) in FORMANTS {
        let resonator = Resonator::new(frequency, bandwidth, SAMPLE_RATE);
        let peak = peak_frequency(&resonator);
        let half_power = resonator.frequency_response(peak) / 2.0_f64.sqrt();
        let below = scan_until(&resonator, peak, -SCAN_STEP, half_power);
        let above = scan_until(&resonator, peak, SCAN_STEP, half_power);
        let width = above - below;
        assert!(
            (width - bandwidth).abs() / bandwidth < TOLERANCE,
            "{frequency} Hz: set {bandwidth} Hz, measured {width} Hz"
        );
    }
}

#[test]
fn resonator_has_unity_gain_at_zero_frequency() {
    for (frequency, bandwidth) in FORMANTS {
        let gain = Resonator::new(frequency, bandwidth, SAMPLE_RATE).frequency_response(0.0);
        assert!((gain - 1.0).abs() < 1e-12, "{frequency} Hz: gain {gain}");
    }
}

#[test]
fn antiresonator_response_is_inverse_of_resonator() {
    for (frequency, bandwidth) in FORMANTS {
        let resonator = Resonator::new(frequency, bandwidth, SAMPLE_RATE);
        let antiresonator = Antiresonator::new(frequency, bandwidth, SAMPLE_RATE);
        for probe in [0.0, 200.0, frequency, 5000.0, 15_000.0] {
            let product =
                resonator.frequency_response(probe) * antiresonator.frequency_response(probe);
            assert!((product - 1.0).abs() < 1e-12);
        }
    }
}

#[test]
fn cascade_response_is_product_of_resonators() {
    let cascade = Cascade::new(&FORMANTS, SAMPLE_RATE);
    let probe = 1234.0;
    let product: f64 = FORMANTS
        .iter()
        .map(|&(frequency, bandwidth)| {
            Resonator::new(frequency, bandwidth, SAMPLE_RATE).frequency_response(probe)
        })
        .product();
    assert!((cascade.frequency_response(probe) - product).abs() < 1e-9 * product);
}

#[test]
fn changing_parameters_keeps_state() {
    let mut gliding = Resonator::new(1000.0, 100.0, SAMPLE_RATE);
    let mut fresh = Resonator::new(1000.0, 100.0, SAMPLE_RATE);
    let input = noise(100);
    for &sample in &input {
        gliding.process_sample(sample);
    }
    gliding.set_parameters(1100.0, 100.0);
    fresh.set_parameters(1100.0, 100.0);
    // A filter that kept its state still rings from the earlier input, a fresh one stays silent.
    assert_ne!(gliding.process_sample(0.0), 0.0);
    assert_eq!(fresh.process_sample(0.0), 0.0);
}

fn peak_frequency(resonator: &Resonator) -> f64 {
    let mut best = (0.0, 0.0);
    let mut probe = 0.0;
    while probe < SAMPLE_RATE / 2.0 {
        let gain = resonator.frequency_response(probe);
        if gain > best.1 {
            best = (probe, gain);
        }
        probe += SCAN_STEP;
    }
    best.0
}

/// Steps away from `start` until the gain falls below `threshold`.
fn scan_until(resonator: &Resonator, start: f64, step: f64, threshold: f64) -> f64 {
    let mut probe = start;
    while resonator.frequency_response(probe) > threshold {
        probe += step;
    }
    probe
}
