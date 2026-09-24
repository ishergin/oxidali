#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSource {
    Unset,
    Manual,
    Sntp,
}

impl TimeSource {
    pub const fn as_str(&self) -> &'static str {
        match self {
            TimeSource::Unset => "unset",
            TimeSource::Manual => "manual",
            TimeSource::Sntp => "sntp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalCivilTime {
    pub minutes_since_midnight: u16,
    pub weekday: u8,
    pub year_day: u16,
    pub utc_offset_minutes: i16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeError {
    NotPlausible,
    InvalidTimezone,
}

pub trait WallClock: Send + Sync {
    fn now_ms(&self) -> Option<u64>;

    fn source(&self) -> TimeSource;

    fn set_manual_ms(&self, unix_ms: u64) -> Result<(), TimeError>;

    fn timezone(&self) -> String;

    fn set_timezone(&self, posix_tz: &str) -> Result<(), TimeError>;

    fn local(&self) -> Option<LocalCivilTime>;
}
