use vowel_bender::oto::OtoFile;
use vowel_bender::quantity::{SampleRate, Time};
use vowel_bender::synthetic::{self, SyntheticBankSettings, SyntheticTruth, TRUTH_FILE_NAME};
use vowel_bender::voicebank::read_wav;

#[test]
fn written_bank_is_consistent() {
    let directory = tempfile::tempdir().unwrap();
    let bank = synthetic::generate(&SyntheticBankSettings::default());

    bank.write(directory.path()).unwrap();

    let oto_bytes = fs_err::read(directory.path().join("oto.ini")).unwrap();
    let oto = OtoFile::parse(&oto_bytes, None);

    assert_eq!(oto.to_bytes().unwrap(), oto_bytes);
    assert_eq!(oto.unparsed_lines().count(), 0);
    assert_eq!(oto.entries().count(), bank.voicebank.samples.len());

    for entry in oto.entries() {
        let (audio, sample_rate) = read_wav(directory.path().join(&entry.file_name)).unwrap();

        assert_eq!(sample_rate, SampleRate::from_hertz(44_100));

        let file_length = sample_rate.time_of_sample(audio.len());
        let end = entry.end(file_length);
        let alias = &entry.alias;

        assert!(entry.offset >= Time::ZERO, "{alias}");
        assert!(entry.overlap < entry.preutterance, "{alias}");
        assert!(entry.preutterance < entry.consonant, "{alias}");
        assert!(entry.offset + entry.consonant < end, "{alias}");
        assert!(end <= file_length, "{alias}");
        assert!(audio.iter().all(|sample| sample.abs() <= 1.0), "{alias}");
    }

    let truth = SyntheticTruth::read(directory.path().join(TRUTH_FILE_NAME)).unwrap();

    assert_eq!(truth, bank.truth);
    assert_eq!(truth.vowels.len(), 5);
}

#[test]
fn bank_has_japanese_gaps() {
    let bank = synthetic::generate(&SyntheticBankSettings::default());
    let aliases: Vec<&str> = bank
        .voicebank
        .samples
        .iter()
        .map(|sample| sample.entry.alias.as_str())
        .collect();

    for present in ["a", "ka", "shi", "chi", "tsu", "fu", "mo"] {
        assert!(aliases.contains(&present), "missing {present}");
    }

    for absent in ["si", "ti", "tu", "hu"] {
        assert!(!aliases.contains(&absent), "unexpected {absent}");
    }
}

#[test]
fn each_vowel_spans_the_pitch_band() {
    let settings = SyntheticBankSettings::default();
    let bank = synthetic::generate(&settings);

    let half_band_ratio = (settings.pitch_spread / 2.0).frequency_ratio();
    let lowest_allowed = settings.base_fundamental_frequency / half_band_ratio;
    let highest_allowed = settings.base_fundamental_frequency * half_band_ratio;

    for vowel in bank.truth.vowels.keys() {
        let pitches: Vec<f64> = bank
            .truth
            .samples
            .iter()
            .filter(|sample| &sample.vowel == vowel)
            .map(|sample| sample.fundamental_frequency.hertz())
            .collect();

        let lowest = pitches.iter().copied().fold(f64::INFINITY, f64::min);
        let highest = pitches.iter().copied().fold(0.0, f64::max);

        assert!(
            lowest >= lowest_allowed.hertz(),
            "{vowel}: lowest {lowest} Hz"
        );
        assert!(
            highest <= highest_allowed.hertz(),
            "{vowel}: highest {highest} Hz"
        );

        // Each vowel covers at least half the band.
        assert!(
            highest / lowest > half_band_ratio,
            "{vowel}: {lowest} to {highest} Hz"
        );
    }
}

#[test]
fn generation_is_deterministic() {
    let settings = SyntheticBankSettings::default();
    let first = synthetic::generate(&settings);
    let second = synthetic::generate(&settings);

    for (one, other) in first
        .voicebank
        .samples
        .iter()
        .zip(&second.voicebank.samples)
    {
        assert_eq!(one.audio, other.audio);
    }
}
