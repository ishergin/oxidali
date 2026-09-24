use core::ffi::{c_char, c_int};
use core::fmt::Write;
use std::sync::OnceLock;

use dali2rust_bsp::log_ring;
use dali2rust_platform::logs::LogLevel;
use esp_idf_svc::log::{EspIdfLogFilter, EspIdfLogger};
use esp_idf_svc::sys::{esp_log_set_vprintf, va_list, vprintf_like_t};

use super::console::ConsoleFillResult;
use super::line::{split_line, trim_tail};

unsafe extern "C" {
    fn vsnprintf(buf: *mut c_char, size: usize, format: *const c_char, args: va_list) -> c_int;
}

const ANSI_SCAN_BYTES: usize = 12;
static PREVIOUS: OnceLock<vprintf_like_t> = OnceLock::new();
static PROJECT_TARGETS: OnceLock<&'static [&'static str]> = OnceLock::new();
static UART_LEVELS: EspIdfLogger<EspIdfLogFilter> = EspIdfLogger::new(EspIdfLogFilter::new());

fn level_of(format: *const c_char) -> Option<LogLevel> {
    // SAFETY: ESP-IDF provides a NUL-terminated static format string.
    let head = unsafe { core::slice::from_raw_parts(format.cast::<u8>(), ANSI_SCAN_BYTES) };
    if head[0] != 0x1b {
        return LogLevel::from_esp_letter(head[0]);
    }
    head.iter()
        .position(|byte| *byte == b'm')
        .and_then(|end| head.get(end + 1).copied())
        .and_then(LogLevel::from_esp_letter)
}

unsafe extern "C" fn capture(format: *const c_char, args: va_list) -> c_int {
    let level = level_of(format).unwrap_or(LogLevel::Info);
    let mut rendered_len = 0usize;
    let queued = super::console::try_write(level, |buffer| {
        // SAFETY: the closure runs once with a valid slot and is the only consumer of the caller's `va_list`.
        let written = unsafe { vsnprintf(buffer.as_mut_ptr().cast(), buffer.len(), format, args) };
        let required = written.max(0) as usize;
        rendered_len = required.min(buffer.len().saturating_sub(1));
        record_c_line(level, &buffer[..rendered_len]);
        ConsoleFillResult {
            len: rendered_len,
            truncated: required >= buffer.len(),
        }
    });
    if queued {
        rendered_len as c_int
    } else {
        0
    }
}

fn record_c_line(level: LogLevel, rendered: &[u8]) {
    let Some(ring) = log_ring::try_global() else {
        return;
    };
    if !ring.accepts(level) {
        return;
    }
    let (target, text) = split_line(trim_tail(rendered));
    ring.record(
        level,
        target,
        text,
        dali2rust_platform::dali::wall_clock_millis(),
    );
}

pub fn install() {
    if PREVIOUS.get().is_some() {
        return;
    }
    let _ = log_ring::global();
    // SAFETY: `capture` has the exact vprintf signature and remains static.
    let previous = unsafe { esp_log_set_vprintf(Some(capture)) };
    let _ = PREVIOUS.set(previous);
}

struct FixedWriter<'a> {
    bytes: &'a mut [u8],
    len: usize,
    truncated: bool,
}

impl<'a> FixedWriter<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self {
            bytes,
            len: 0,
            truncated: false,
        }
    }
}

impl Write for FixedWriter<'_> {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let take = (self.bytes.len() - self.len).min(text.len());
        self.bytes[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
        self.truncated |= take != text.len();
        Ok(())
    }
}

fn uart_accepts(metadata: &log::Metadata) -> bool {
    if option_env!("DALI2RUST_ESP_VERBOSE").is_some() {
        return true;
    }
    metadata.level() <= log::Level::Warn
        || PROJECT_TARGETS
            .get()
            .is_some_and(|targets| targets.contains(&metadata.target()))
}

struct TeeLogger;

impl log::Log for TeeLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        uart_accepts(metadata) || log_ring::facade_accepts(metadata.level())
    }

    fn log(&self, record: &log::Record) {
        let keep_history = log_ring::facade_accepts(record.level());
        if !uart_accepts(record.metadata()) {
            if keep_history {
                log_ring::record_facade(record);
            }
            return;
        }
        if !write_record_to_console(record, keep_history) && keep_history {
            log_ring::record_facade(record);
        }
    }

    fn flush(&self) {}
}

fn write_record_to_console(record: &log::Record, keep_history: bool) -> bool {
    let level = log_ring::facade_level(record.level());
    let letter = match record.level() {
        log::Level::Error => 'E',
        log::Level::Warn => 'W',
        log::Level::Info => 'I',
        log::Level::Debug => 'D',
        log::Level::Trace => 'V',
    };
    // SAFETY: pure FFI clock read.
    let stamp = unsafe { esp_idf_svc::sys::esp_log_timestamp() };
    super::console::try_write(level, |buffer| {
        let mut line = FixedWriter::new(buffer);
        let _ = write!(line, "{letter} ({stamp}) {}: ", record.target());
        let text_start = line.len;
        let _ = write!(line, "{}", record.args());
        if keep_history {
            if let Some(ring) = log_ring::try_global() {
                ring.record(
                    level,
                    record.target().as_bytes(),
                    &line.bytes[text_start..line.len],
                    dali2rust_platform::dali::wall_clock_millis(),
                );
            }
        }
        let _ = line.write_str("\n");
        ConsoleFillResult {
            len: line.len,
            truncated: line.truncated,
        }
    })
}

static TEE: TeeLogger = TeeLogger;

pub fn install_facade(
    project_targets: &'static [&'static str],
) -> &'static EspIdfLogger<EspIdfLogFilter> {
    let _ = log_ring::global();
    let _ = PROJECT_TARGETS.set(project_targets);
    if log::set_logger(&TEE).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }
    &UART_LEVELS
}
