use std::path::Path;

use vowel_bender::oto::{Encoding, OtoEntry, OtoError, OtoFile};

fn shift_jis(text: &str) -> Vec<u8> {
    let (bytes, _, had_errors) = encoding_rs::SHIFT_JIS.encode(text);
    assert!(!had_errors);
    bytes.into_owned()
}

/// A Shift-JIS oto.ini with kana aliases, CRLF endings, a blank line, a malformed line and no trailing newline.
fn shift_jis_fixture() -> Vec<u8> {
    let mut bytes = shift_jis(
        "_あ.wav=- あ,50,120,-400,80,20\r\n\
         _かきくけこ.wav=か,1234.5,150,-500,100,30\r\n\
         \r\n\
         _かきくけこ.wav=き,1800,150,500.25,100,-10\r\n\
         this line is not an entry\r\n\
         _か.wav=,0,0,0,0,0\r\n",
    );
    // A NEC-selected IBM extension kanji, which a decode and re-encode rewrites to its IBM extension bytes.
    bytes.extend_from_slice(b"_\xED\x40.wav=");
    bytes.extend(shift_jis("く,1,2,3,4,5"));
    bytes
}

#[test]
fn shift_jis_round_trip_is_identical() {
    let original = shift_jis_fixture();
    let oto = OtoFile::parse(&original, None);
    assert_eq!(oto.encoding(), Encoding::ShiftJis);
    assert_eq!(oto.to_bytes().unwrap(), original);
    assert_eq!(oto.lines().len(), 7);
    assert_eq!(oto.entries().count(), 5);
    assert_eq!(oto.unparsed_line_indices(), vec![4]);
}

#[test]
fn naive_shift_jis_round_trip_changes_extension_characters() {
    // This is why the oto module keeps raw bytes instead of decoding and encoding every line.
    let original = b"\xED\x40";
    let (decoded, _) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(original);
    assert_ne!(shift_jis(&decoded), original);
}

#[test]
fn shift_jis_entries_parse() {
    let oto = OtoFile::parse(&shift_jis_fixture(), Some(Encoding::ShiftJis));
    let entries: Vec<&OtoEntry> = oto.entries().collect();
    assert_eq!(
        *entries[0],
        OtoEntry {
            file_name: "_あ.wav".to_string(),
            alias: "- あ".to_string(),
            offset: 50.0,
            consonant: 120.0,
            cutoff: -400.0,
            preutterance: 80.0,
            overlap: 20.0,
        }
    );
    assert_eq!(entries[1].offset, 1234.5);
    assert_eq!(entries[2].cutoff, 500.25);
    assert_eq!(entries[2].overlap, -10.0);
    assert_eq!(entries[3].alias, "");
    assert_eq!(entries[4].alias, "く");
}

#[test]
fn utf8_round_trips_are_identical() {
    let body = "åå.wav=åå,10,20,-30,15,5\nöö.wav=öö,10,20,-30,15,5\n";
    let with_byte_order_mark = [b"\xEF\xBB\xBF".as_slice(), body.as_bytes()].concat();
    for original in [body.as_bytes().to_vec(), with_byte_order_mark] {
        let oto = OtoFile::parse(&original, None);
        assert_eq!(oto.encoding(), Encoding::Utf8);
        assert_eq!(oto.to_bytes().unwrap(), original);
        assert_eq!(oto.entries().count(), 2);
    }
}

#[test]
fn editing_an_entry_changes_only_that_line() {
    let original = shift_jis_fixture();
    let mut oto = OtoFile::parse(&original, None);
    let mut entry = oto.lines()[1].entry().unwrap().clone();
    entry.alias = "が".to_string();
    entry.offset = 1200.0;
    oto.set_entry(1, entry);

    let expected = String::from_utf8_lossy(&original).replacen(
        &*String::from_utf8_lossy(&shift_jis("か,1234.5,")),
        &String::from_utf8_lossy(&shift_jis("が,1200,")),
        1,
    );
    let written = oto.to_bytes().unwrap();
    assert_eq!(String::from_utf8_lossy(&written), expected);

    let reparsed = OtoFile::parse(&written, Some(Encoding::ShiftJis));
    assert_eq!(reparsed.lines()[1].entry().unwrap().alias, "が");
}

#[test]
fn pushing_an_entry_ends_the_previous_line() {
    let mut oto = OtoFile::parse(b"a.wav=a,1,2,3,4,5", None);
    oto.push_entry(OtoEntry {
        file_name: "i.wav".to_string(),
        alias: "i".to_string(),
        offset: 1.0,
        consonant: 2.0,
        cutoff: -3.5,
        preutterance: 4.0,
        overlap: 5.0,
    });
    assert_eq!(
        oto.to_bytes().unwrap(),
        b"a.wav=a,1,2,3,4,5\r\ni.wav=i,1,2,-3.5,4,5\r\n"
    );
}

#[test]
fn unencodable_edit_is_an_error() {
    let mut oto = OtoFile::parse(&shift_jis_fixture(), None);
    let mut entry = oto.lines()[0].entry().unwrap().clone();
    entry.alias = "åå".to_string();
    oto.set_entry(0, entry);
    assert!(matches!(
        oto.to_bytes(),
        Err(OtoError::Unencodable {
            line_index: 0,
            encoding: Encoding::ShiftJis
        })
    ));
}

/// Round trips every oto.ini under `$VOWEL_BENDER_TEST_BANKS`.
/// Real banks stay local, so the test does nothing when the variable is unset.
#[test]
fn local_banks_round_trip() {
    let Some(root) = std::env::var_os("VOWEL_BENDER_TEST_BANKS") else {
        eprintln!("VOWEL_BENDER_TEST_BANKS is unset, skipping");
        return;
    };
    let mut paths = Vec::new();
    find_oto_files(Path::new(&root), &mut paths);
    assert!(
        !paths.is_empty(),
        "no oto.ini under {}",
        Path::new(&root).display()
    );
    for path in paths {
        let original = std::fs::read(&path).unwrap();
        let oto = OtoFile::parse(&original, None);
        assert_eq!(oto.to_bytes().unwrap(), original, "{}", path.display());
    }
}

fn find_oto_files(directory: &Path, paths: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_oto_files(&path, paths);
        } else if path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("oto.ini"))
        {
            paths.push(path);
        }
    }
}
