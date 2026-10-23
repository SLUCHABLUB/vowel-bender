use std::path::{Path, PathBuf};

use vowel_bender::oto::{Cutoff, Encoding, EntryParseError, OtoEntry, OtoError, OtoFile};
use vowel_bender::quantity::Time;

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

fn entry(alias: &str, cutoff: Cutoff) -> OtoEntry {
    OtoEntry {
        file_name: format!("{alias}.wav"),
        alias: alias.to_string(),
        offset: Time::from_milliseconds(1.0),
        consonant: Time::from_milliseconds(2.0),
        cutoff,
        preutterance: Time::from_milliseconds(4.0),
        overlap: Time::from_milliseconds(5.0),
    }
}

#[test]
fn shift_jis_round_trip_is_identical() {
    let original = shift_jis_fixture();
    let oto = OtoFile::parse(&original, None);

    assert_eq!(oto.encoding(), Encoding::ShiftJis);
    assert_eq!(oto.to_bytes().unwrap(), original);
    assert_eq!(oto.lines().len(), 7);
    assert_eq!(oto.entries().count(), 5);

    let unparsed: Vec<_> = oto.unparsed_lines().collect();

    assert_eq!(unparsed.len(), 1);

    let (line_number, line) = unparsed[0];

    assert_eq!(line_number, 5);
    assert_eq!(
        line.original_bytes(),
        Some(b"this line is not an entry".as_slice())
    );
    assert_eq!(line.parse_error(), Some(&EntryParseError::MissingEquals));
}

#[test]
fn naive_shift_jis_round_trip_changes_extension_characters() {
    // This is why the oto module keeps original bytes instead of decoding and encoding every line.
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
            offset: Time::from_milliseconds(50.0),
            consonant: Time::from_milliseconds(120.0),
            cutoff: Cutoff::AfterOffset(Time::from_milliseconds(400.0)),
            preutterance: Time::from_milliseconds(80.0),
            overlap: Time::from_milliseconds(20.0),
        }
    );
    assert_eq!(entries[1].offset, Time::from_milliseconds(1234.5));
    assert_eq!(
        entries[2].cutoff,
        Cutoff::BeforeEndOfFile(Time::from_milliseconds(500.25))
    );
    assert_eq!(entries[2].overlap, Time::from_milliseconds(-10.0));
    assert_eq!(entries[3].alias, "");
    assert_eq!(entries[4].alias, "く");
}

#[test]
fn missing_fields_mean_zero() {
    // OpenUtau reads missing fields as zero too.
    let oto = OtoFile::parse(b"a.wav=a,10\r\ni.wav=i\r\nu.wav=\r\n", None);
    let entries: Vec<&OtoEntry> = oto.entries().collect();

    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].offset, Time::from_milliseconds(10.0));
    assert_eq!(entries[0].consonant, Time::from_milliseconds(0.0));
    assert_eq!(entries[1].offset, Time::from_milliseconds(0.0));
    assert_eq!(entries[2].alias, "");
}

#[test]
fn invalid_lines_say_why() {
    let oto = OtoFile::parse(b"a.wav=a,1,2,3,4,5,6\r\ni.wav=i,1,two,3,4,5\r\n", None);
    let errors: Vec<_> = oto
        .unparsed_lines()
        .map(|(_, line)| line.parse_error().unwrap().clone())
        .collect();

    assert_eq!(errors[0], EntryParseError::TooManyFields { count: 7 });
    assert!(matches!(
        &errors[1],
        EntryParseError::InvalidTime { field: "consonant", text, .. } if text == "two"
    ));
}

#[test]
fn cutoff_sign_survives_editing() {
    let mut oto = OtoFile::new(Encoding::Utf8);

    oto.push_entry(entry(
        "a",
        Cutoff::AfterOffset(Time::from_milliseconds(0.0)),
    ));
    oto.push_entry(entry(
        "i",
        Cutoff::BeforeEndOfFile(Time::from_milliseconds(0.0)),
    ));

    let written = oto.to_bytes().unwrap();

    assert_eq!(written, b"a.wav=a,1,2,-0,4,5\r\ni.wav=i,1,2,0,4,5\r\n");

    let reparsed = OtoFile::parse(&written, None);
    let cutoffs: Vec<Cutoff> = reparsed.entries().map(|entry| entry.cutoff).collect();

    assert_eq!(
        cutoffs,
        [
            Cutoff::AfterOffset(Time::from_milliseconds(0.0)),
            Cutoff::BeforeEndOfFile(Time::from_milliseconds(0.0)),
        ]
    );
}

#[test]
fn edited_times_are_written_in_shortest_form() {
    // 63.7 ms does not survive a conversion to seconds and back exactly.
    let mut edited = entry("a", Cutoff::BeforeEndOfFile(Time::from_milliseconds(127.4)));

    edited.offset = Time::from_milliseconds(63.7);

    let mut oto = OtoFile::new(Encoding::Utf8);

    oto.push_entry(edited);

    assert_eq!(oto.to_bytes().unwrap(), b"a.wav=a,63.7,2,127.4,4,5\r\n");
}

#[test]
fn utf8_round_trips_are_identical() {
    let body = "åå.wav=åå,10,20,-30,15,5\nöö.wav=öö,10,20,-30,15,5\n";
    let with_byte_order_mark = ["\u{FEFF}".as_bytes(), body.as_bytes()].concat();

    for original in [body.as_bytes().to_vec(), with_byte_order_mark] {
        let oto = OtoFile::parse(&original, None);

        assert_eq!(oto.encoding(), Encoding::Utf8);
        assert_eq!(oto.to_bytes().unwrap(), original);
        assert_eq!(oto.entries().count(), 2);
    }
}

#[test]
fn charset_declaration_overrides_detection() {
    // Plain ASCII is valid UTF-8, so without the declaration this would be read as UTF-8.
    let oto = OtoFile::parse(b"#Charset:shift_jis\r\na.wav=a,1,2,3,4,5\r\n", None);

    assert_eq!(oto.encoding(), Encoding::ShiftJis);
}

#[test]
fn editing_an_entry_changes_only_that_line() {
    let original = shift_jis_fixture();
    let mut oto = OtoFile::parse(&original, None);

    let line = &mut oto.lines_mut()[1];
    let mut entry = line.entry().unwrap().clone();

    entry.alias = "が".to_string();
    entry.offset = Time::from_milliseconds(1200.0);
    line.set_entry(entry);

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

    oto.push_entry(entry(
        "i",
        Cutoff::AfterOffset(Time::from_milliseconds(3.5)),
    ));

    assert_eq!(
        oto.to_bytes().unwrap(),
        b"a.wav=a,1,2,3,4,5\r\ni.wav=i,1,2,-3.5,4,5\r\n"
    );
}

#[test]
fn unencodable_edit_is_an_error() {
    let mut oto = OtoFile::parse(&shift_jis_fixture(), None);

    let line = &mut oto.lines_mut()[0];
    let mut entry = line.entry().unwrap().clone();

    entry.alias = "åå".to_string();
    line.set_entry(entry);

    assert!(matches!(
        oto.to_bytes(),
        Err(OtoError::Unencodable {
            line_number: 1,
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
        let original = fs_err::read(&path).unwrap();
        let oto = OtoFile::parse(&original, None);

        assert_eq!(oto.to_bytes().unwrap(), original, "{}", path.display());
    }
}

fn find_oto_files(directory: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs_err::read_dir(directory) else {
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
