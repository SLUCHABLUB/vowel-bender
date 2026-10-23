use rand::{RngExt, SeedableRng};
use rand_xoshiro::Xoshiro256PlusPlus;
use vowel_bender::filter::{Antiresonator, Cascade, Formant, Resonator};
use vowel_bender::quantity::{Frequency, Gain, SampleRate};

const SAMPLE_RATE: SampleRate = SampleRate::from_hertz(44_100);

/// Formants with bandwidth well below frequency, and frequency well below Nyquist,
/// where a digital resonator's −3 dB width closely matches its bandwidth setting.
const FORMANTS: [Formant; 5] = [
    formant(500.0, 60.0),
    formant(1000.0, 100.0),
    formant(1500.0, 90.0),
    formant(2500.0, 150.0),
    formant(3500.0, 200.0),
];

/// Step of the frequency scans.
const SCAN_STEP: Frequency = Frequency::from_hertz(0.1);

const fn formant(frequency: f64, bandwidth: f64) -> Formant {
    Formant {
        frequency: Frequency::from_hertz(frequency),
        bandwidth: Frequency::from_hertz(bandwidth),
    }
}

fn noise(length: usize) -> Vec<f64> {
    let mut random = Xoshiro256PlusPlus::seed_from_u64(7);

    (0..length)
        .map(|_| random.random_range(-1.0..1.0))
        .collect()
}

#[test]
fn antiresonator_undoes_resonator() {
    let input = noise(44_100);

    for formant in FORMANTS {
        let mut signal = input.clone();

        Resonator::new(formant, SAMPLE_RATE).process_buffer(&mut signal);
        Antiresonator::new(formant, SAMPLE_RATE).process_buffer(&mut signal);

        let largest_error = input
            .iter()
            .zip(&signal)
            .map(|(original, restored)| (original - restored).abs())
            .fold(0.0, f64::max);

        assert!(
            largest_error < 1e-9,
            "{}: error {largest_error}",
            formant.frequency
        );
    }
}

#[test]
fn resonator_peaks_at_its_frequency() {
    for formant in FORMANTS {
        let resonator = Resonator::new(formant, SAMPLE_RATE);
        let peak = peak_frequency(&resonator);
        let relative_error = ((peak - formant.frequency) / formant.frequency).abs();

        assert!(
            relative_error < 0.01,
            "set {}, peak at {peak}",
            formant.frequency
        );
    }
}

#[test]
fn resonator_width_matches_bandwidth() {
    // The match is approximate, closest for narrow bandwidths far below Nyquist.
    const TOLERANCE: f64 = 0.05;

    for formant in FORMANTS {
        let resonator = Resonator::new(formant, SAMPLE_RATE);
        let peak = peak_frequency(&resonator);
        let half_power = resonator.frequency_response(peak).linear() / 2.0_f64.sqrt();
        let half_power = Gain::from_linear(half_power);

        let below = scan_until(&resonator, peak, -SCAN_STEP, half_power);
        let above = scan_until(&resonator, peak, SCAN_STEP, half_power);

        let width = above - below;
        let relative_error = ((width - formant.bandwidth) / formant.bandwidth).abs();

        assert!(
            relative_error < TOLERANCE,
            "{}: set {}, measured {width}",
            formant.frequency,
            formant.bandwidth
        );
    }
}

#[test]
fn resonator_has_unity_gain_at_zero_frequency() {
    for formant in FORMANTS {
        let gain = Resonator::new(formant, SAMPLE_RATE).frequency_response(Frequency::ZERO);

        assert!(
            (gain.linear() - 1.0).abs() < 1e-12,
            "{}: gain {gain}",
            formant.frequency
        );
    }
}

#[test]
fn antiresonator_response_is_inverse_of_resonator() {
    for formant in FORMANTS {
        let resonator = Resonator::new(formant, SAMPLE_RATE);
        let antiresonator = Antiresonator::new(formant, SAMPLE_RATE);

        let probes =
            [0.0, 200.0, formant.frequency.hertz(), 5000.0, 15_000.0].map(Frequency::from_hertz);

        for probe in probes {
            let product =
                resonator.frequency_response(probe) * antiresonator.frequency_response(probe);

            assert!((product.linear() - 1.0).abs() < 1e-12);
        }
    }
}

#[test]
fn cascade_response_is_product_of_resonators() {
    let cascade = Cascade::new(&FORMANTS, SAMPLE_RATE);
    let probe = Frequency::from_hertz(1234.0);

    let product: Gain = FORMANTS
        .iter()
        .map(|&formant| Resonator::new(formant, SAMPLE_RATE).frequency_response(probe))
        .product();

    let difference = (cascade.frequency_response(probe).linear() - product.linear()).abs();

    assert!(difference < 1e-9 * product.linear());
}

#[test]
fn changing_formant_keeps_state() {
    let mut gliding = Resonator::new(formant(1000.0, 100.0), SAMPLE_RATE);
    let mut fresh = Resonator::new(formant(1000.0, 100.0), SAMPLE_RATE);

    for sample in noise(100) {
        gliding.process_sample(sample);
    }

    gliding.set_formant(formant(1100.0, 100.0));
    fresh.set_formant(formant(1100.0, 100.0));

    // A filter that kept its state still rings from the earlier input, a fresh one stays silent.
    assert_ne!(gliding.process_sample(0.0), 0.0);
    assert_eq!(fresh.process_sample(0.0), 0.0);
}

fn peak_frequency(resonator: &Resonator) -> Frequency {
    let mut best_frequency = Frequency::ZERO;
    let mut best_gain = Gain::SILENCE;
    let mut probe = Frequency::ZERO;

    while probe < SAMPLE_RATE.nyquist_frequency() {
        let gain = resonator.frequency_response(probe);

        if gain > best_gain {
            best_frequency = probe;
            best_gain = gain;
        }

        probe += SCAN_STEP;
    }

    best_frequency
}

/// Steps away from `start` until the gain falls below `threshold`.
fn scan_until(
    resonator: &Resonator,
    start: Frequency,
    step: Frequency,
    threshold: Gain,
) -> Frequency {
    let mut probe = start;

    while resonator.frequency_response(probe) > threshold {
        probe += step;
    }

    probe
}
