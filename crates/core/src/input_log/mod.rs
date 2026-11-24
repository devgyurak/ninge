// SPDX-License-Identifier: Apache-2.0
//! The input log: what the host gave the machine, one frame per line, as text
//! (`docs/contributing/input-log.md`).
//!
//! Pure like the savestate format: text in, [`Input`] values out, and the reverse, with no IO.
//! The host writes the header and then one line per frame it ran; a replay parses them back.
//! Guest-facing input never panics here: malformed text is an [`InputLogError`] with the line it
//! is on (AGENTS.md §2 rule 11).

use crate::cart::SaveType;
use crate::input::{Buttons, Input, Touch};
use crate::savestate::fnv1a64;

/// The format version this build reads and writes. A log of another version is refused.
pub const FORMAT_VERSION: u32 = 1;

/// The word every log starts with.
const MAGIC: &str = "ninge-input";

/// The lower screen's last row; a touch below it is out of range (`keypad.md`).
const TOUCH_MAX_Y: u8 = 191;

/// The button names in bit order, A at bit 0.
const BUTTON_NAMES: [(&str, Buttons); 12] = [
    ("A", Buttons::A),
    ("B", Buttons::B),
    ("SELECT", Buttons::SELECT),
    ("START", Buttons::START),
    ("RIGHT", Buttons::RIGHT),
    ("LEFT", Buttons::LEFT),
    ("UP", Buttons::UP),
    ("DOWN", Buttons::DOWN),
    ("R", Buttons::R),
    ("L", Buttons::L),
    ("X", Buttons::X),
    ("Y", Buttons::Y),
];

/// What a log belongs to: the program and the save memory it started with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogHeader {
    /// The ROM's header CRC16 (`Header::crc`). Never the ROM's name, path, or bytes.
    pub rom_crc: u16,
    /// The save chip the run started with.
    pub save_type: SaveType,
    /// [`save_hash`] of the save memory as the machine held it before frame 1.
    pub save_hash: u64,
}

/// Why a log was refused. `line` is the 1-based line of the text, comments and blanks included.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("input log line {line}: {kind}")]
pub struct InputLogError {
    /// The line the problem is on; for [`LogErrorKind::MissingHeader`] it is the line that
    /// should have been the header, or 0 for a text with no lines at all.
    pub line: usize,
    /// What is wrong with it.
    pub kind: LogErrorKind,
}

/// What is wrong with a line of a log.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LogErrorKind {
    /// The first line that is not a comment is not a header.
    #[error("expected the `ninge-input` header first")]
    MissingHeader,
    /// The header has no version, or the version is not a number.
    #[error("the header has no readable version")]
    BadVersion,
    /// The log is of a format version this build does not read.
    #[error("format version {found} is not supported (this build reads version {expected})")]
    UnsupportedVersion {
        /// The version in the log.
        found: u32,
        /// The version this build reads.
        expected: u32,
    },
    /// The header lacks a required key.
    #[error("the header has no `{0}=`")]
    MissingHeaderKey(&'static str),
    /// A key this version does not know.
    #[error("unknown field `{0}`")]
    UnknownKey(String),
    /// A key given twice on one line.
    #[error("field `{0}` is given twice")]
    DuplicateKey(String),
    /// A key whose value cannot be read: missing, empty, malformed, or given to a flag.
    #[error("bad value for `{key}`: `{value}`")]
    BadValue {
        /// The key.
        key: &'static str,
        /// The text after the `=`, or empty when there was none.
        value: String,
    },
    /// A button name that is not one of the twelve, or one named twice.
    #[error("unknown or repeated button `{0}`")]
    BadButton(String),
    /// A touch outside the lower screen, 0-255 by 0-191.
    #[error("touch {x},{y} is outside the screen (x 0-255, y 0-191)")]
    TouchOutOfRange {
        /// The column as written.
        x: u32,
        /// The row as written.
        y: u32,
    },
    /// A frame line that does not start with `f=<n>`.
    #[error("a frame line starts with `f=<n>`")]
    MissingFrameNumber,
    /// A frame number that is not the previous one plus one.
    #[error("frame number {found} where {expected} is next")]
    FrameOutOfOrder {
        /// The number the line should have.
        expected: u32,
        /// The number it has.
        found: u32,
    },
}

/// Why a log does not belong to the program and save a host is about to replay it against.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MismatchError {
    /// The log was recorded for another ROM.
    #[error("the log is for a ROM with header CRC {log:04x}, not {actual:04x}")]
    Rom {
        /// The CRC in the log.
        log: u16,
        /// The CRC of the ROM at hand.
        actual: u16,
    },
    /// The log started with another kind of save chip.
    #[error("the log started with a `{}` save, not `{}`", .log.name(), .actual.name())]
    SaveType {
        /// The type in the log.
        log: SaveType,
        /// The type at hand.
        actual: SaveType,
    },
    /// The log started with other save memory.
    #[error("the log started with other save memory (hash {log:016x}, not {actual:016x})")]
    SaveHash {
        /// The hash in the log.
        log: u64,
        /// The hash of the save memory at hand.
        actual: u64,
    },
}

impl LogHeader {
    /// Whether a replay against this ROM and save is the run the log recorded.
    ///
    /// # Errors
    /// The first of ROM, save type, and save hash that differs.
    pub fn check(
        &self,
        rom_crc: u16,
        save_type: SaveType,
        save_hash: u64,
    ) -> Result<(), MismatchError> {
        match (
            self.rom_crc == rom_crc,
            self.save_type == save_type,
            self.save_hash == save_hash,
        ) {
            (false, _, _) => Err(MismatchError::Rom {
                log: self.rom_crc,
                actual: rom_crc,
            }),
            (_, false, _) => Err(MismatchError::SaveType {
                log: self.save_type,
                actual: save_type,
            }),
            (_, _, false) => Err(MismatchError::SaveHash {
                log: self.save_hash,
                actual: save_hash,
            }),
            _ => Ok(()),
        }
    }
}

/// The hash a header carries for the save memory: FNV-1a 64, the hash the savestate and the
/// golden frames use.
pub fn save_hash(save_data: &[u8]) -> u64 {
    fnv1a64(save_data)
}

/// The header line, newline included.
pub fn encode_header(header: &LogHeader) -> String {
    format!(
        "{MAGIC} {FORMAT_VERSION} rom={:04x} save={}:{:016x}\n",
        header.rom_crc,
        header.save_type.name(),
        header.save_hash
    )
}

/// The line for frame `number` (1 for the first), newline included. A field at its default is
/// left out, so a frame with nothing happening is just `f=<n>`.
pub fn encode_frame(number: u32, input: &Input) -> String {
    let mut line = format!("f={number}");
    if input.buttons.bits() != 0 {
        let names: Vec<&str> = BUTTON_NAMES
            .iter()
            .filter(|(_, button)| input.buttons.contains(*button))
            .map(|(name, _)| *name)
            .collect();
        line.push_str(&format!(" b={}", names.join(",")));
    }
    if let Some(touch) = input.touch {
        line.push_str(&format!(" t={},{}", touch.x, touch.y));
    }
    if input.hinge_closed {
        line.push_str(" hinge");
    }
    if input.mic {
        line.push_str(" mic");
    }
    if input.wall_time != 0 {
        line.push_str(&format!(" wall={}", input.wall_time));
    }
    line.push('\n');
    line
}

/// What one line of a log said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLine {
    /// The header.
    Header(LogHeader),
    /// A frame's input.
    Frame(Input),
}

/// A line-by-line reader: feed it the lines of a log in order. It keeps what the next line needs
/// (whether the header came, which frame is next) so a host can read a pipe as well as a file.
#[derive(Clone, Debug, Default)]
pub struct LogReader {
    line: usize,
    header_seen: bool,
    next_frame: u32,
}

impl LogReader {
    /// A reader at the start of a log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the next line (with or without its newline). A comment or a blank line gives
    /// `Ok(None)`.
    ///
    /// # Errors
    /// The line is refused; the reader's position still moves past it.
    pub fn feed(&mut self, text: &str) -> Result<Option<LogLine>, InputLogError> {
        self.line += 1;
        let text = text.trim();
        if text.is_empty() || text.starts_with('#') {
            return Ok(None);
        }
        let at = |kind| InputLogError {
            line: self.line,
            kind,
        };
        if self.header_seen {
            parse_frame(text, self.next_frame).map_err(at).map(|input| {
                // A log of 2^32 frames is out of reach; saturating keeps the reader total.
                self.next_frame = self.next_frame.saturating_add(1);
                Some(LogLine::Frame(input))
            })
        } else {
            parse_header(text).map_err(at).map(|header| {
                self.header_seen = true;
                self.next_frame = 1;
                Some(LogLine::Header(header))
            })
        }
    }
}

/// A whole log, read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedLog {
    /// What the log belongs to.
    pub header: LogHeader,
    /// The input of each frame, frame 1 first.
    pub frames: Vec<Input>,
    /// Whether a last line with no newline was dropped (a log cut off mid-write).
    pub dropped_partial_line: bool,
}

/// Reads a whole log. Only newline-terminated lines count: a last line without a newline is
/// dropped and reported in [`ParsedLog::dropped_partial_line`], not refused.
///
/// # Errors
/// The first line that is refused, or [`LogErrorKind::MissingHeader`] for a text with no header.
pub fn parse(text: &str) -> Result<ParsedLog, InputLogError> {
    let mut reader = LogReader::new();
    let mut header = None;
    let mut frames = Vec::new();
    let mut dropped_partial_line = false;
    for raw in text.split_inclusive('\n') {
        if !raw.ends_with('\n') {
            dropped_partial_line = true;
            break;
        }
        match reader.feed(raw)? {
            Some(LogLine::Header(h)) => header = Some(h),
            Some(LogLine::Frame(input)) => frames.push(input),
            None => {}
        }
    }
    header
        .map(|header| ParsedLog {
            header,
            frames,
            dropped_partial_line,
        })
        .ok_or(InputLogError {
            line: reader.line + usize::from(dropped_partial_line),
            kind: LogErrorKind::MissingHeader,
        })
}

fn parse_header(text: &str) -> Result<LogHeader, LogErrorKind> {
    let mut words = text.split_whitespace();
    (words.next() == Some(MAGIC))
        .then_some(())
        .ok_or(LogErrorKind::MissingHeader)?;
    let found = words
        .next()
        .and_then(decimal)
        .ok_or(LogErrorKind::BadVersion)?;
    (found == FORMAT_VERSION)
        .then_some(())
        .ok_or(LogErrorKind::UnsupportedVersion {
            found,
            expected: FORMAT_VERSION,
        })?;
    let mut rom_crc = None;
    let mut save = None;
    for word in words {
        let (key, value) = split_field(word);
        match key {
            "rom" => set_once(&mut rom_crc, key, parse_rom(value)?)?,
            "save" => set_once(&mut save, key, parse_save(value)?)?,
            _ => return Err(LogErrorKind::UnknownKey(key.to_string())),
        }
    }
    let rom_crc = rom_crc.ok_or(LogErrorKind::MissingHeaderKey("rom"))?;
    let (save_type, save_hash) = save.ok_or(LogErrorKind::MissingHeaderKey("save"))?;
    Ok(LogHeader {
        rom_crc,
        save_type,
        save_hash,
    })
}

fn parse_rom(value: Option<&str>) -> Result<u16, LogErrorKind> {
    value
        .and_then(|v| hex(v, 4))
        .and_then(|crc| u16::try_from(crc).ok())
        .ok_or_else(|| bad("rom", value))
}

fn parse_save(value: Option<&str>) -> Result<(SaveType, u64), LogErrorKind> {
    value
        .and_then(|v| v.split_once(':'))
        .and_then(|(name, digits)| Some((SaveType::from_name(name)?, hex(digits, 16)?)))
        .ok_or_else(|| bad("save", value))
}

/// A frame line; `expected` is the number its `f=` must carry.
fn parse_frame(text: &str, expected: u32) -> Result<Input, LogErrorKind> {
    let mut words = text.split_whitespace();
    let (key, value) = split_field(words.next().unwrap_or_default());
    (key == "f")
        .then_some(())
        .ok_or(LogErrorKind::MissingFrameNumber)?;
    let found = value.and_then(decimal).ok_or_else(|| bad("f", value))?;
    (found == expected)
        .then_some(())
        .ok_or(LogErrorKind::FrameOutOfOrder { expected, found })?;
    let mut buttons = None;
    let mut touch = None;
    let mut hinge = None;
    let mut mic = None;
    let mut wall = None;
    for word in words {
        let (key, value) = split_field(word);
        match key {
            "b" => set_once(&mut buttons, "b", parse_buttons(value)?)?,
            "t" => set_once(&mut touch, "t", parse_touch(value)?)?,
            "hinge" => set_once(&mut hinge, "hinge", parse_flag("hinge", value)?)?,
            "mic" => set_once(&mut mic, "mic", parse_flag("mic", value)?)?,
            "wall" => set_once(
                &mut wall,
                "wall",
                value.and_then(decimal).ok_or_else(|| bad("wall", value))?,
            )?,
            "f" => return Err(LogErrorKind::DuplicateKey("f".to_string())),
            _ => return Err(LogErrorKind::UnknownKey(key.to_string())),
        }
    }
    Ok(Input::new()
        .with_buttons(buttons.unwrap_or_default())
        .with_touch(touch)
        .with_hinge_closed(hinge.is_some())
        .with_mic(mic.is_some())
        .with_wall_time(wall.unwrap_or(0)))
}

fn parse_buttons(value: Option<&str>) -> Result<Buttons, LogErrorKind> {
    value
        .filter(|v| !v.is_empty())
        .ok_or_else(|| bad("b", value))?
        .split(',')
        .try_fold(Buttons::empty(), |held, name| {
            BUTTON_NAMES
                .iter()
                .find(|(known, _)| *known == name)
                .filter(|(_, button)| !held.contains(*button))
                .map(|(_, button)| held | *button)
                .ok_or_else(|| LogErrorKind::BadButton(name.to_string()))
        })
}

fn parse_touch(value: Option<&str>) -> Result<Touch, LogErrorKind> {
    let (x, y) = value
        .and_then(|v| v.split_once(','))
        .and_then(|(x, y)| Some((decimal(x)?, decimal(y)?)))
        .ok_or_else(|| bad("t", value))?;
    match (u8::try_from(x), u8::try_from(y)) {
        (Ok(x), Ok(y)) if y <= TOUCH_MAX_Y => Ok(Touch { x, y }),
        _ => Err(LogErrorKind::TouchOutOfRange { x, y }),
    }
}

/// A key with no value, like `hinge`; a value on it is malformed.
fn parse_flag(key: &'static str, value: Option<&str>) -> Result<(), LogErrorKind> {
    value.map_or(Ok(()), |_| Err(bad(key, value)))
}

fn set_once<T>(slot: &mut Option<T>, key: &str, value: T) -> Result<(), LogErrorKind> {
    match slot.replace(value) {
        Some(_) => Err(LogErrorKind::DuplicateKey(key.to_string())),
        None => Ok(()),
    }
}

fn split_field(word: &str) -> (&str, Option<&str>) {
    word.split_once('=')
        .map_or((word, None), |(key, value)| (key, Some(value)))
}

fn bad(key: &'static str, value: Option<&str>) -> LogErrorKind {
    LogErrorKind::BadValue {
        key,
        value: value.unwrap_or_default().to_string(),
    }
}

/// A decimal number of ASCII digits only: `str::parse` would also take a leading `+`.
fn decimal(text: &str) -> Option<u32> {
    text.bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| text.parse().ok())
        .flatten()
}

/// Exactly `digits` hex digits, as a number: `from_str_radix` would also take a leading `+`.
fn hex(text: &str, digits: usize) -> Option<u64> {
    (text.len() == digits && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u64::from_str_radix(text, 16).ok())
        .flatten()
}

#[cfg(test)]
mod tests;
