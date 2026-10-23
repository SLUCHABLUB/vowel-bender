//! Reading and writing oto.ini, the sample index of a UTAU voicebank.
//!
//! Each line has the form `file.wav=alias,offset,consonant,cutoff,preutterance,overlap`, with times in milliseconds.
//! The format has no formal specification.
//! This module follows how OpenUtau reads it, in `ParseOto` and `GetOtoDeclaredEncoding` of
//! [`VoicebankLoader.cs`](https://github.com/stakira/OpenUtau/blob/master/OpenUtau.Core/Classic/VoicebankLoader.cs),
//! and how a resampler interprets the timings, in
//! [straycat-rs](https://github.com/UtaUtaUtau/straycat-rs/blob/master/src/resample.rs).
//!
//! # Why every line keeps its original bytes
//!
//! Decoding a line and encoding it again does not always give back the same bytes:
//!
//! - Shift-JIS has duplicate codes for some characters.
//!   The NEC-selected IBM extensions (lead bytes `0xED` and `0xEE`) repeat the IBM extensions (`0xFA` to `0xFC`),
//!   and the [WHATWG Shift_JIS encoder](https://encoding.spec.whatwg.org/#index-shift_jis-pointer),
//!   which `encoding_rs` implements, always picks the IBM codes.
//! - Bytes that are invalid in the file's encoding decode to `U+FFFD`, which cannot be encoded back.
//! - Numbers have many spellings, such as `50`, `50.0` and `+50`, and formatting a parsed number picks one of them.
//!
//! Banks are often tuned by hand, and their files are the user's data.
//! So an unedited line is written back exactly as it was read, and only added or edited lines are encoded.
//! Editing one alias then changes one line of the file, and test 2 of the design, a byte-identical round trip, holds by construction.

use std::borrow::Cow;
use std::io;
use std::num::ParseFloatError;
use std::path::Path;

use fs_err as fs;
use thiserror::Error;

use crate::quantity::Time;

const BYTE_ORDER_MARK: &[u8] = "\u{FEFF}".as_bytes();

/// OpenUtau looks for this declaration in the first lines of an oto.ini, and it overrides the voicebank's encoding.
const CHARSET_DECLARATION: &[u8] = b"#Charset:";

/// How many lines OpenUtau searches for a [`CHARSET_DECLARATION`].
const CHARSET_DECLARATION_LINES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
pub enum Encoding {
    #[display("Shift-JIS")]
    ShiftJis,
    #[display("UTF-8")]
    Utf8,
}

impl Encoding {
    /// Picks the encoding a `#Charset:` declaration names.
    /// Without one, picks UTF-8 when the bytes start with a byte order mark or are valid UTF-8, and Shift-JIS otherwise.
    ///
    /// OpenUtau uses Shift-JIS unless the voicebank's character.yaml or a `#Charset:` declaration says otherwise.
    /// Guessing from the bytes only matters for files that are valid UTF-8 and not plain ASCII.
    #[must_use]
    pub fn detect(bytes: &[u8]) -> Self {
        if let Some(declared) = Self::declared(bytes) {
            return declared;
        }

        if bytes.starts_with(BYTE_ORDER_MARK) || std::str::from_utf8(bytes).is_ok() {
            Encoding::Utf8
        } else {
            Encoding::ShiftJis
        }
    }

    /// The encoding named by a `#Charset:` declaration, if there is one this module knows.
    fn declared(bytes: &[u8]) -> Option<Self> {
        let declaration = bytes
            .split(|&byte| byte == b'\n')
            .take(CHARSET_DECLARATION_LINES)
            .find_map(|line| line.strip_prefix(CHARSET_DECLARATION))?;

        let name = String::from_utf8_lossy(declaration)
            .trim()
            .to_ascii_lowercase();

        match name.as_str() {
            "utf-8" | "utf8" => Some(Encoding::Utf8),
            "shift_jis" | "shift-jis" | "sjis" => Some(Encoding::ShiftJis),
            _ => None,
        }
    }

    fn decode(self, bytes: &[u8]) -> Cow<'_, str> {
        match self {
            Encoding::ShiftJis => {
                let (string, _error) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(bytes);

                string
            }
            Encoding::Utf8 => String::from_utf8_lossy(bytes),
        }
    }

    fn encode(self, text: &str) -> Option<Vec<u8>> {
        match self {
            Encoding::ShiftJis => {
                let (bytes, _, error) = encoding_rs::SHIFT_JIS.encode(text);

                (!error).then(|| bytes.into_owned())
            }
            Encoding::Utf8 => Some(text.as_bytes().to_vec()),
        }
    }
}

/// Where a sample ends.
/// The sign of the number in the file says which end it is measured from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cutoff {
    /// The sample ends this long after the offset.
    /// Written as a negative number.
    AfterOffset(Time),
    /// The sample ends this long before the end of the file.
    /// Written as a non-negative number.
    BeforeEndOfFile(Time),
}

impl Cutoff {
    fn from_file_value(value: Time) -> Self {
        // `is_sign_negative` keeps `-0` as a zero length after the offset, so it survives a round trip.
        if value.seconds().is_sign_negative() {
            Cutoff::AfterOffset(-value)
        } else {
            Cutoff::BeforeEndOfFile(value)
        }
    }

    fn file_value(self) -> Time {
        match self {
            Cutoff::AfterOffset(length) => -length,
            Cutoff::BeforeEndOfFile(margin) => margin,
        }
    }
}

/// One sample's alias and timing.
#[derive(Debug, Clone, PartialEq)]
pub struct OtoEntry {
    pub file_name: String,
    /// The name lyrics are matched against.
    /// OpenUtau uses the file name without its extension when the alias is empty.
    pub alias: String,
    /// Where the sample starts, from the start of the file.
    pub offset: Time,
    /// Length of the fixed region that is never stretched, from the offset.
    pub consonant: Time,
    pub cutoff: Cutoff,
    /// Where the note starts, from the offset.
    pub preutterance: Time,
    /// Length of the crossfade with the previous note, from the offset.
    pub overlap: Time,
}

/// Why a line is not a valid entry.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EntryParseError {
    #[error("there is no `=` between the file name and the alias")]
    MissingEquals,
    #[error("there are {count} fields after `=`, but at most 6 are allowed")]
    TooManyFields { count: usize },
    #[error("the {field} `{text}` is not a number")]
    InvalidTime {
        field: &'static str,
        text: String,
        #[source]
        source: ParseFloatError,
    },
}

impl OtoEntry {
    /// Parses one line.
    /// Like OpenUtau, missing and empty time fields mean zero.
    fn parse(text: &str) -> Result<Self, EntryParseError> {
        const FIELD_NAMES: [&str; 6] = [
            "alias",
            "offset",
            "consonant",
            "cutoff",
            "preutterance",
            "overlap",
        ];

        let (file_name, fields) = text.split_once('=').ok_or(EntryParseError::MissingEquals)?;

        let fields: Vec<_> = fields.split(',').collect();

        if fields.len() > FIELD_NAMES.len() {
            return Err(EntryParseError::TooManyFields {
                count: fields.len(),
            });
        }

        let time = |index: usize| {
            let text = fields.get(index).copied().unwrap_or_default();

            parse_time(text).map_err(|source| EntryParseError::InvalidTime {
                field: FIELD_NAMES[index],
                text: text.to_string(),
                source,
            })
        };

        Ok(Self {
            file_name: file_name.to_string(),
            alias: fields[0].to_string(),
            offset: time(1)?,
            consonant: time(2)?,
            cutoff: Cutoff::from_file_value(time(3)?),
            preutterance: time(4)?,
            overlap: time(5)?,
        })
    }

    /// Where the sample ends, from the start of a file `file_length` long.
    #[must_use]
    pub fn end(&self, file_length: Time) -> Time {
        match self.cutoff {
            Cutoff::AfterOffset(length) => self.offset + length,
            Cutoff::BeforeEndOfFile(margin) => file_length - margin,
        }
    }
}

/// Writes the entry as an oto.ini line, without a line ending.
impl std::fmt::Display for OtoEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let OtoEntry {
            file_name,
            alias,
            offset,
            consonant,
            cutoff,
            preutterance,
            overlap,
        } = self;

        let offset = format_time(*offset);
        let consonant = format_time(*consonant);
        let cutoff = format_time(cutoff.file_value());
        let preutterance = format_time(*preutterance);
        let overlap = format_time(*overlap);

        write!(
            formatter,
            "{file_name}={alias},{offset},{consonant},{cutoff},{preutterance},{overlap}"
        )
    }
}

/// Parses a time field, which is in milliseconds.
fn parse_time(field: &str) -> Result<Time, ParseFloatError> {
    let field = field.trim();

    let milliseconds = if field.is_empty() {
        0.0
    } else {
        field.parse()?
    };

    Ok(Time::from_milliseconds(milliseconds))
}

/// Writes a time in milliseconds, with the fewest decimals that parse back to the same time.
///
/// Times are stored in seconds, and converting milliseconds to seconds and back is not always exact:
/// 63.7 ms comes back as 63.699999999999996 ms.
/// Rounding to the shortest string that parses back to the same time writes 63.7 again.
fn format_time(time: Time) -> String {
    const MAXIMUM_DECIMALS: usize = 17;

    let milliseconds = time.milliseconds();

    for decimals in 0..=MAXIMUM_DECIMALS {
        let text = format!("{milliseconds:.decimals$}");

        let parsed = parse_time(&text).is_ok_and(|parsed| parsed == time);

        if parsed {
            return text;
        }
    }

    milliseconds.to_string()
}

/// What a line read from a file holds.
#[derive(Debug, Clone, PartialEq)]
enum LineContent {
    Blank,
    Entry(OtoEntry),
    Invalid(EntryParseError),
}

#[derive(Debug, Clone)]
enum LineState {
    /// Read from a file, and written back as the same bytes.
    /// The module documentation explains why.
    Original {
        /// The line's bytes, without its line ending.
        bytes: Vec<u8>,
        content: LineContent,
    },
    /// Added or edited, and encoded from the entry when written.
    Edited(OtoEntry),
}

/// One line of an oto.ini.
#[derive(Debug, Clone)]
pub struct OtoLine {
    state: LineState,
    line_ending: LineEnding,
}

impl OtoLine {
    /// The line's bytes as read, without its line ending, or `None` if the line was added or edited.
    #[must_use]
    pub fn original_bytes(&self) -> Option<&[u8]> {
        match &self.state {
            LineState::Original { bytes, .. } => Some(bytes),
            LineState::Edited(_) => None,
        }
    }

    /// The entry, or `None` for blank and invalid lines.
    #[must_use]
    pub fn entry(&self) -> Option<&OtoEntry> {
        match &self.state {
            LineState::Original {
                content: LineContent::Entry(entry),
                ..
            }
            | LineState::Edited(entry) => Some(entry),
            LineState::Original { .. } => None,
        }
    }

    /// Why the line is not a valid entry, or `None` for blank lines and entries.
    #[must_use]
    pub fn parse_error(&self) -> Option<&EntryParseError> {
        match &self.state {
            LineState::Original {
                content: LineContent::Invalid(error),
                ..
            } => Some(error),
            LineState::Original { .. } | LineState::Edited(_) => None,
        }
    }

    /// Replaces the line with an entry, which is encoded when the file is written.
    pub fn set_entry(&mut self, entry: OtoEntry) {
        self.state = LineState::Edited(entry);
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
enum LineEnding {
    None,
    LineFeed,
    /// UTAU is a Windows program, and banks made for it use Windows line endings.
    #[default]
    CarriageReturnLineFeed,
}

impl LineEnding {
    fn is_none(self) -> bool {
        self == LineEnding::None
    }

    fn as_bytes(self) -> &'static [u8] {
        match self {
            LineEnding::None => b"",
            LineEnding::LineFeed => b"\n",
            LineEnding::CarriageReturnLineFeed => b"\r\n",
        }
    }
}

#[derive(Debug, Error)]
pub enum OtoError {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// An added or edited line holds a character the file's encoding cannot represent, such as å in Shift-JIS.
    #[error("line {line_number} cannot be encoded as {encoding}")]
    Unencodable {
        line_number: usize,
        encoding: Encoding,
    },
}

/// A whole oto.ini.
#[derive(Debug, Clone)]
pub struct OtoFile {
    encoding: Encoding,
    has_byte_order_mark: bool,
    lines: Vec<OtoLine>,
}

impl OtoFile {
    #[must_use]
    pub fn new(encoding: Encoding) -> Self {
        Self {
            encoding,
            has_byte_order_mark: false,
            lines: Vec::new(),
        }
    }

    /// Parses an oto.ini, detecting its encoding when `encoding` is `None`.
    #[must_use]
    pub fn parse(bytes: &[u8], encoding: Option<Encoding>) -> Self {
        let encoding = encoding.unwrap_or_else(|| Encoding::detect(bytes));

        let has_byte_order_mark = encoding == Encoding::Utf8 && bytes.starts_with(BYTE_ORDER_MARK);

        let body = if has_byte_order_mark {
            &bytes[BYTE_ORDER_MARK.len()..]
        } else {
            bytes
        };

        // Shift-JIS trail bytes are never 0x0A or 0x0D, so splitting on raw bytes is safe in both encodings.
        let mut lines = Vec::new();
        let mut remaining = body;

        while !remaining.is_empty() {
            let newline_index = remaining.iter().position(|&byte| byte == b'\n');

            let line: &[u8];
            let line_ending: LineEnding;
            let rest: &[u8];

            if let Some(mut newline_index) = newline_index {
                rest = &remaining[newline_index + 1..];

                let carriage_return = newline_index > 0 && remaining[newline_index - 1] == b'\r';

                if carriage_return {
                    newline_index -= 1;
                    line_ending = LineEnding::CarriageReturnLineFeed;
                } else {
                    line_ending = LineEnding::LineFeed;
                }

                line = &remaining[..newline_index];
            } else {
                line = remaining;
                line_ending = LineEnding::None;
                rest = &remaining[remaining.len()..];
            }

            let text = encoding.decode(line);

            let content = if text.trim().is_empty() {
                LineContent::Blank
            } else {
                match OtoEntry::parse(&text) {
                    Ok(entry) => LineContent::Entry(entry),
                    Err(error) => LineContent::Invalid(error),
                }
            };

            lines.push(OtoLine {
                state: LineState::Original {
                    bytes: line.to_vec(),
                    content,
                },
                line_ending,
            });

            remaining = rest;
        }

        Self {
            encoding,
            has_byte_order_mark,
            lines,
        }
    }

    /// Reads and parses an oto.ini, detecting its encoding when `encoding` is `None`.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read.
    pub fn read<P>(path: P, encoding: Option<Encoding>) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        let bytes = fs::read(path.as_ref())?;

        Ok(Self::parse(&bytes, encoding))
    }

    #[must_use]
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    #[must_use]
    pub fn has_byte_order_mark(&self) -> bool {
        self.has_byte_order_mark
    }

    #[must_use]
    pub fn lines(&self) -> &[OtoLine] {
        &self.lines
    }

    /// The lines, for editing them with [`OtoLine::set_entry`].
    pub fn lines_mut(&mut self) -> &mut [OtoLine] {
        &mut self.lines
    }

    pub fn entries(&self) -> impl Iterator<Item = &OtoEntry> {
        self.lines.iter().filter_map(OtoLine::entry)
    }

    /// Lines that are neither blank nor a valid entry, with their line numbers counted from 1.
    pub fn unparsed_lines(&self) -> impl Iterator<Item = (usize, &OtoLine)> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.parse_error().is_some())
            .map(|(index, line)| (index + 1, line))
    }

    /// Appends an entry as a new line.
    pub fn push_entry(&mut self, entry: OtoEntry) {
        if let Some(last) = self.lines.last_mut()
            && last.line_ending.is_none()
        {
            last.line_ending = LineEnding::default();
        }

        self.lines.push(OtoLine {
            state: LineState::Edited(entry),
            line_ending: LineEnding::default(),
        });
    }

    /// Encodes the file, keeping the original bytes of every unedited line.
    ///
    /// # Errors
    ///
    /// Returns [`OtoError::Unencodable`] when an added or edited line holds a character the file's encoding cannot represent.
    pub fn to_bytes(&self) -> Result<Vec<u8>, OtoError> {
        let mut bytes = Vec::new();

        if self.has_byte_order_mark {
            bytes.extend_from_slice(BYTE_ORDER_MARK);
        }

        for (index, line) in self.lines.iter().enumerate() {
            match &line.state {
                LineState::Original {
                    bytes: original_bytes,
                    ..
                } => bytes.extend_from_slice(original_bytes),
                LineState::Edited(entry) => {
                    let encoded =
                        self.encoding
                            .encode(&entry.to_string())
                            .ok_or(OtoError::Unencodable {
                                line_number: index + 1,
                                encoding: self.encoding,
                            })?;

                    bytes.extend_from_slice(&encoded);
                }
            }

            bytes.extend_from_slice(line.line_ending.as_bytes());
        }

        Ok(bytes)
    }

    /// Encodes the file and writes it to `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when an added or edited line cannot be encoded or the file cannot be written.
    pub fn write<P>(&self, path: P) -> Result<(), OtoError>
    where
        P: AsRef<Path>,
    {
        let bytes = self.to_bytes()?;

        fs::write(path.as_ref(), bytes)?;

        Ok(())
    }
}
