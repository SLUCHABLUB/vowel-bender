//! Reading and writing oto.ini, the sample index of a voicebank.
//!
//! Each line has the form `file.wav=alias,offset,consonant,cutoff,preutterance,overlap`, with times in milliseconds.
//! Every line keeps its original bytes, and only edited lines are encoded again.
//! That makes an unedited round trip byte-identical,
//! even for characters a Shift-JIS decode and re-encode would not preserve.

use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

const BYTE_ORDER_MARK: &[u8] = "\u{FEFF}".as_bytes();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    ShiftJis,
    Utf8,
}

impl Encoding {
    /// Picks UTF-8 when the bytes start with a byte order mark or are valid UTF-8, and Shift-JIS otherwise.
    #[must_use]
    pub fn detect(bytes: &[u8]) -> Self {
        if bytes.starts_with(BYTE_ORDER_MARK) || std::str::from_utf8(bytes).is_ok() {
            Encoding::Utf8
        } else {
            Encoding::ShiftJis
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

// TODO: Derive this.
impl fmt::Display for Encoding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Encoding::ShiftJis => formatter.write_str("Shift-JIS"),
            Encoding::Utf8 => formatter.write_str("UTF-8"),
        }
    }
}

/// One sample's alias and timing, all times in milliseconds.
#[derive(Debug, Clone, PartialEq)]
pub struct OtoEntry {
    pub file_name: String,
    pub alias: String,
    /// Where the sample starts, from the start of the file.
    pub offset: f64,
    /// Length of the fixed region that is never stretched, from the offset.
    pub consonant: f64,
    /// Where the sample ends.
    /// Negative values are a length from the offset, positive values are a margin from the end of the file.
    pub cutoff: f64,
    /// Where the note starts, from the offset.
    pub preutterance: f64,
    /// Length of the crossfade with the previous note, from the offset.
    pub overlap: f64,
}

impl OtoEntry {
    // TODO: Improve the error handling.
    fn parse(text: &str) -> Option<Self> {
        let (file_name, fields) = text.split_once('=')?;

        let fields: Vec<&str> = fields.split(',').collect();

        let [alias, offset, consonant, cutoff, preutterance, overlap] = fields.as_slice() else {
            return None;
        };

        Some(Self {
            file_name: file_name.to_string(),
            alias: alias.to_string(),
            offset: parse_time(offset)?,
            consonant: parse_time(consonant)?,
            cutoff: parse_time(cutoff)?,
            preutterance: parse_time(preutterance)?,
            overlap: parse_time(overlap)?,
        })
    }

    // TODO: Use display.
    fn format(&self) -> String {
        let OtoEntry {
            file_name,
            alias,
            offset,
            consonant,
            cutoff,
            preutterance,
            overlap,
        } = self;
        format!("{file_name}={alias},{offset},{consonant},{cutoff},{preutterance},{overlap}")
    }

    // TODO: Use new-types for quantities.
    /// Where the sample ends, in milliseconds from the start of a file `file_length` milliseconds long.
    #[must_use]
    pub fn end(&self, file_length: f64) -> f64 {
        if self.cutoff < 0.0 {
            self.offset - self.cutoff
        } else {
            file_length - self.cutoff
        }
    }
}

/// Empty time fields occur in hand-edited files and mean zero.
fn parse_time(field: &str) -> Option<f64> {
    let field = field.trim();

    if field.is_empty() {
        Some(0.0)
    } else {
        field.parse().ok()
    }
}

/// One line of an oto.ini, with its original bytes.
#[derive(Debug, Clone)]
pub struct OtoLine {
    // TODO: Why is this here.
    raw: Vec<u8>,
    line_ending: LineEnding,
    entry: Option<OtoEntry>,
    modified: bool,
}

impl OtoLine {
    /// The line's bytes as read, without its line ending.
    #[must_use]
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// The parsed entry, or `None` for blank or malformed lines.
    #[must_use]
    pub fn entry(&self) -> Option<&OtoEntry> {
        self.entry.as_ref()
    }

    pub fn is_blank(&self) -> bool {
        self.raw.iter().all(u8::is_ascii_whitespace)
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, Default)]
enum LineEnding {
    None,
    LineFeed,
    // Utau apparently expect these, so have them as the default.
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

// TODO: Use thiserror.
#[derive(Debug)]
pub enum OtoError {
    Io(io::Error),
    /// An edited line holds a character the file's encoding cannot represent, such as å in Shift-JIS.
    Unencodable {
        line_index: usize,
        encoding: Encoding,
    },
}

impl fmt::Display for OtoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OtoError::Io(error) => write!(formatter, "{error}"),
            OtoError::Unencodable {
                line_index,
                encoding,
            } => {
                write!(
                    formatter,
                    "line {} cannot be encoded as {encoding}",
                    line_index + 1
                )
            }
        }
    }
}

impl std::error::Error for OtoError {}

impl From<io::Error> for OtoError {
    fn from(error: io::Error) -> Self {
        OtoError::Io(error)
    }
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

            lines.push(OtoLine {
                raw: line.to_vec(),
                line_ending,
                entry: OtoEntry::parse(&encoding.decode(line)),
                modified: false,
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
        Ok(Self::parse(&fs::read(path)?, encoding))
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

    pub fn entries(&self) -> impl Iterator<Item = &OtoEntry> {
        self.lines.iter().filter_map(OtoLine::entry)
    }

    /// Indices of lines that are neither blank nor a valid entry.
    pub fn unparsed_lines(&self) -> impl Iterator<Item = &OtoLine> {
        self.lines
            .iter()
            .filter(|line| line.entry.is_none() && !line.is_blank())
    }

    // TODO: This panics.
    /// Replaces the line at `line_index` with an entry; it is encoded again when written.
    pub fn set_entry(&mut self, line_index: usize, entry: OtoEntry) {
        let line = &mut self.lines[line_index];
        line.entry = Some(entry);
        line.modified = true;
    }

    /// Appends an entry as a new line.
    pub fn push_entry(&mut self, entry: OtoEntry) {
        if let Some(last) = self.lines.last_mut()
            && last.line_ending.is_none()
        {
            last.line_ending = LineEnding::default();
        }

        self.lines.push(OtoLine {
            raw: Vec::new(),
            line_ending: LineEnding::default(),
            entry: Some(entry),
            modified: true,
        });
    }

    /// Encodes the file, keeping the original bytes of every unedited line.
    ///
    /// # Errors
    ///
    /// Returns [`OtoError::Unencodable`] when an edited line holds a character the file's encoding cannot represent.
    pub fn to_bytes(&self) -> Result<Vec<u8>, OtoError> {
        let mut bytes = Vec::new();

        if self.has_byte_order_mark {
            bytes.extend_from_slice(BYTE_ORDER_MARK);
        }

        for (line_index, line) in self.lines.iter().enumerate() {
            if let Some(entry) = &line.entry
                && line.modified
            {
                let encoded =
                    self.encoding
                        .encode(&entry.format())
                        .ok_or(OtoError::Unencodable {
                            line_index,
                            encoding: self.encoding,
                        })?;
                bytes.extend_from_slice(&encoded);
            } else {
                bytes.extend_from_slice(&line.raw);
            }

            bytes.extend_from_slice(line.line_ending.as_bytes());
        }

        Ok(bytes)
    }

    /// Encodes the file and writes it to `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when an edited line cannot be encoded or the file cannot be written.
    pub fn write(&self, path: &Path) -> Result<(), OtoError> {
        fs::write(path, self.to_bytes()?)?;
        Ok(())
    }
}
