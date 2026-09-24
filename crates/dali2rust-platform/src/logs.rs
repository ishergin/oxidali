#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

#[derive(Debug, Default)]
pub struct ConsoleLogCounters {
    pub dropped: core::sync::atomic::AtomicU32,
    pub truncated: core::sync::atomic::AtomicU32,
    pub busy: core::sync::atomic::AtomicU32,
    pub unavailable: core::sync::atomic::AtomicU32,
    pub uart_errors: core::sync::atomic::AtomicU32,
}

impl ConsoleLogCounters {
    pub const fn new() -> Self {
        Self {
            dropped: core::sync::atomic::AtomicU32::new(0),
            truncated: core::sync::atomic::AtomicU32::new(0),
            busy: core::sync::atomic::AtomicU32::new(0),
            unavailable: core::sync::atomic::AtomicU32::new(0),
            uart_errors: core::sync::atomic::AtomicU32::new(0),
        }
    }
}

impl LogLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "error" => Some(Self::Error),
            "warn" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }

    pub const fn from_esp_letter(letter: u8) -> Option<Self> {
        match letter {
            b'E' => Some(Self::Error),
            b'W' => Some(Self::Warn),
            b'I' => Some(Self::Info),
            b'D' => Some(Self::Debug),
            b'V' => Some(Self::Trace),
            _ => None,
        }
    }
}

pub const LOG_TARGET_BYTES: usize = 32;

pub const LOG_TEXT_BYTES: usize = 144;

const TRUNCATION_MARKER: &[u8; 3] = b"...";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogLine {
    pub at_ms: u64,
    pub seq: u32,
    pub level: LogLevel,
    pub target_len: u8,
    pub text_len: u8,
    pub target: [u8; LOG_TARGET_BYTES],
    pub text: [u8; LOG_TEXT_BYTES],
}

impl LogLine {
    pub fn new(at_ms: u64, seq: u32, level: LogLevel, target: &[u8], text: &[u8]) -> Self {
        let mut line = Self {
            at_ms,
            seq,
            level,
            target_len: 0,
            text_len: 0,
            target: [0; LOG_TARGET_BYTES],
            text: [0; LOG_TEXT_BYTES],
        };
        line.target_len = copy_truncated(&mut line.target, target, false);
        line.text_len = copy_truncated(&mut line.text, text, true);
        line
    }

    pub fn target_str(&self) -> &str {
        valid_prefix(&self.target[..self.target_len as usize])
    }

    pub fn text_str(&self) -> &str {
        valid_prefix(&self.text[..self.text_len as usize])
    }
}

fn copy_truncated(slot: &mut [u8], source: &[u8], mark: bool) -> u8 {
    let room = slot.len();
    if source.len() <= room {
        slot[..source.len()].copy_from_slice(source);
        return source.len() as u8;
    }
    slot.copy_from_slice(&source[..room]);
    if mark && room >= TRUNCATION_MARKER.len() {
        slot[room - TRUNCATION_MARKER.len()..].copy_from_slice(TRUNCATION_MARKER);
    }
    room as u8
}

fn valid_prefix(bytes: &[u8]) -> &str {
    match core::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            let valid = &bytes[..error.valid_up_to()];
            core::str::from_utf8(valid).unwrap_or("")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LogLevel, LogLine, LOG_TEXT_BYTES};

    #[test]
    fn a_level_filter_is_a_number_comparison_in_the_esp_idf_direction() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Error <= LogLevel::Warn);
        assert!(LogLevel::Info > LogLevel::Warn);
    }

    #[test]
    fn an_unknown_level_name_is_refused_rather_than_defaulted() {
        assert_eq!(LogLevel::parse("warn"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::parse("waming"), None);
        assert_eq!(LogLevel::parse("WARN"), None);
    }

    #[test]
    fn the_esp_format_letter_is_read_without_formatting_anything() {
        assert_eq!(LogLevel::from_esp_letter(b'W'), Some(LogLevel::Warn));
        assert_eq!(LogLevel::from_esp_letter(b'V'), Some(LogLevel::Trace));
        assert_eq!(LogLevel::from_esp_letter(b'?'), None);
    }

    #[test]
    fn an_over_long_message_keeps_its_head_and_says_it_was_cut() {
        let long = "a".repeat(LOG_TEXT_BYTES + 40);
        let line = LogLine::new(7, 1, LogLevel::Info, b"dali2rust::x", long.as_bytes());

        assert_eq!(line.text_len as usize, LOG_TEXT_BYTES);
        assert!(
            line.text_str().ends_with("..."),
            "a cut line must read as cut"
        );
        assert_eq!(line.target_str(), "dali2rust::x");
    }

    #[test]
    fn a_truncated_multibyte_character_is_dropped_not_replaced() {
        let text = "я".repeat(LOG_TEXT_BYTES);
        let line = LogLine::new(0, 0, LogLevel::Info, b"t", text.as_bytes());

        assert!(!line.text_str().contains('\u{fffd}'));
        assert!(line.text_str().len() <= LOG_TEXT_BYTES);
    }

    #[test]
    fn a_line_is_plain_copyable_data_the_size_a_psram_arena_assumes() {
        assert_eq!(core::mem::size_of::<LogLine>(), 192);
        assert!(!core::mem::needs_drop::<LogLine>());
    }
}
