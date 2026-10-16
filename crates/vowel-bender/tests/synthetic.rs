use vowel_bender::oto::OtoFile;
use vowel_bender::synthetic::{self, SyntheticBankSettings, SyntheticTruth, TRUTH_FILE_NAME};
use vowel_bender::voicebank::read_wav;

#[test]
fn written_bank_is_consistent() {
    let directory = tempfile::tempdir().unwrap();
    let bank = synthetic::generate(&SyntheticBankSettings::default());
    bank.write(directory.path()).unwrap();

    let oto_bytes = std::fs::read(directory.path().join("oto.ini")).unwrap();
    let oto = OtoFile::parse(&oto_bytes, None);
    assert_eq!(oto.to_bytes().unwrap(), oto_bytes);
    assert!(oto.unparsed_line_indices().is_empty());
    assert_eq!(oto.entries().count(), bank.voicebank.samples.len());

    for entry in oto.entries() {
        let (audio, sample_rate) = read_wav(&directory.path().join(&entry.file_name)).unwrap();
        assert_eq!(sample_rate, 44_100);
        let file_length = audio.len() as f64 * 1000.0 / f64::from(sample_rate);
        let end = entry.end(file_length);
        assert!(entry.offset >= 0.0, "{}", entry.alias);
        assert!(entry.overlap < entry.preutterance, "{}", entry.alias);
        assert!(entry.preutterance < entry.consonant, "{}", entry.alias);
        assert!(entry.offset + entry.consonant < end, "{}", entry.alias);
        assert!(end <= file_length, "{}", entry.alias);
        assert!(audio.iter().all(|sample| sample.abs() <= 1.0));
    }

    let truth = SyntheticTruth::read(&directory.path().join(TRUTH_FILE_NAME)).unwrap();
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
    let ratio = 2.0_f64.powf(settings.pitch_spread_semitones / 24.0);
    for vowel in bank.truth.vowels.keys() {
        let pitches: Vec<f64> = bank
            .truth
            .samples
            .iter()
            .filter(|sample| &sample.vowel == vowel)
            .map(|sample| sample.fundamental_frequency)
            .collect();
        let lowest = pitches.iter().copied().fold(f64::INFINITY, f64::min);
        let highest = pitches.iter().copied().fold(0.0, f64::max);
        assert!(lowest >= settings.base_fundamental_frequency / ratio);
        assert!(highest <= settings.base_fundamental_frequency * ratio);
        // Each vowel covers at least half the band.
        assert!(
            highest / lowest > ratio,
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
