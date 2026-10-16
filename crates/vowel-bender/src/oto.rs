//! Reading and writing oto.ini, a voicebank's sample index.
//!
//! Each line has the form `file.wav=alias,offset,consonant,cutoff,preutterance,overlap`, with times in milliseconds.
//! Every line keeps its original bytes, and only edited lines are encoded again.
//! That makes an unedited round trip byte-identical,
//! even for characters a Shift-JIS decode and re-encode would not preserve.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

const BYTE_ORDER_MARK: &[u8] = b"\xEF\xBB\xBF";

/// The line ending used for lines this module adds, as UTAU tools expect.
const DEFAULT_LINE_ENDING: &[u8] = b"\r\n";

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

    fn decode(self, bytes: &[u8]) -> String {
        match self {
            Encoding::ShiftJis => encoding_rs::SHIFT_JIS
                .decode_without_bom_handling(bytes)
                .0
                .into_owned(),
            Encoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        }
    }

    fn encode(self, text: &str) -> Option<Vec<u8>> {
        match self {
            Encoding::ShiftJis => {
                let (bytes, _, had_errors) = encoding_rs::SHIFT_JIS.encode(text);
                (!had_errors).then(|| bytes.into_owned())
            }
            Encoding::Utf8 => Some(text.as_bytes().to_vec()),
        }
    }
}

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

    fn format(&self) -> String {
        format!(
            "{}={},{},{},{},{},{}",
            self.file_name,
            self.alias,
            self.offset,
            self.consonant,
            self.cutoff,
            self.preutterance,
            self.overlap
        )
    }

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
    raw: Vec<u8>,
    line_ending: Vec<u8>,
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
            let (line, line_ending, rest) = match remaining.iter().position(|&byte| byte == b'\n') {
                Some(newline) => {
                    let ending_start = if newline > 0 && remaining[newline - 1] == b'\r' {
                        newline - 1
                    } else {
                        newline
                    };
                    (
                        &remaining[..ending_start],
                        &remaining[ending_start..=newline],
                        &remaining[newline + 1..],
                    )
                }
                None => (
                    remaining,
                    &remaining[remaining.len()..],
                    &remaining[remaining.len()..],
                ),
            };
            lines.push(OtoLine {
                raw: line.to_vec(),
                line_ending: line_ending.to_vec(),
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
    pub fn read(path: &Path, encoding: Option<Encoding>) -> io::Result<Self> {
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
    #[must_use]
    pub fn unparsed_line_indices(&self) -> Vec<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.entry.is_none() && !line.is_blank())
            .map(|(index, _)| index)
            .collect()
    }

    /// Replaces the line at `line_index` with an entry; it is encoded again when written.
    pub fn set_entry(&mut self, line_index: usize, entry: OtoEntry) {
        let line = &mut self.lines[line_index];
        line.entry = Some(entry);
        line.modified = true;
    }

    /// Appends an entry as a new line.
    pub fn push_entry(&mut self, entry: OtoEntry) {
        if let Some(last) = self.lines.last_mut()
            && last.line_ending.is_empty()
        {
            last.line_ending = DEFAULT_LINE_ENDING.to_vec();
        }
        self.lines.push(OtoLine {
            raw: Vec::new(),
            line_ending: DEFAULT_LINE_ENDING.to_vec(),
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
            match (&line.entry, line.modified) {
                (Some(entry), true) => {
                    let encoded =
                        self.encoding
                            .encode(&entry.format())
                            .ok_or(OtoError::Unencodable {
                                line_index,
                                encoding: self.encoding,
                            })?;
                    bytes.extend_from_slice(&encoded);
                }
                _ => bytes.extend_from_slice(&line.raw),
            }
            bytes.extend_from_slice(&line.line_ending);
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
