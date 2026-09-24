use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;

use dali2rust_contracts::msg::{fixed_text_96, FixedText96};
use dali2rust_platform::firmware::{FirmwareError, FirmwareSlot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtaPhase {
    Idle = 0,
    Downloading = 1,
    Finishing = 2,
    ReadyToReboot = 3,
    Failed = 4,
}

impl OtaPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Downloading => "downloading",
            Self::Finishing => "finishing",
            Self::ReadyToReboot => "ready_to_reboot",
            Self::Failed => "failed",
        }
    }

    const fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Downloading,
            2 => Self::Finishing,
            3 => Self::ReadyToReboot,
            4 => Self::Failed,
            _ => Self::Idle,
        }
    }

    pub const fn accepts_new_update(self) -> bool {
        matches!(self, Self::Idle | Self::Failed)
    }
}

#[derive(Debug, Clone)]
pub struct OtaSnapshot {
    pub phase: OtaPhase,
    pub downloaded_bytes: u32,
    pub total_bytes: u32,
    pub error: Option<FirmwareError>,
    pub url: FixedText96,
    pub slot: FirmwareSlot,
}

impl OtaSnapshot {
    pub fn percent(&self) -> Option<u8> {
        if self.total_bytes == 0 {
            return None;
        }
        let done = u64::from(self.downloaded_bytes) * 100 / u64::from(self.total_bytes);
        Some(done.min(100) as u8)
    }
}

#[derive(Debug)]
pub struct OtaState {
    phase: AtomicU8,
    downloaded: AtomicU32,
    total: AtomicU32,
    error: AtomicU8,
    url: Mutex<FixedText96>,
}

const NO_ERROR: u8 = u8::MAX;

impl Default for OtaState {
    fn default() -> Self {
        Self {
            phase: AtomicU8::new(OtaPhase::Idle as u8),
            downloaded: AtomicU32::new(0),
            total: AtomicU32::new(0),
            error: AtomicU8::new(NO_ERROR),
            url: Mutex::new(FixedText96::new()),
        }
    }
}

const fn error_code(error: FirmwareError) -> u8 {
    match error {
        FirmwareError::BadUrl => 0,
        FirmwareError::Fetch => 1,
        FirmwareError::NoSlot => 2,
        FirmwareError::Write => 3,
        FirmwareError::InvalidImage => 4,
        FirmwareError::Cancelled => 5,
        FirmwareError::NoMemory => 6,
    }
}

const fn error_from_code(code: u8) -> Option<FirmwareError> {
    match code {
        0 => Some(FirmwareError::BadUrl),
        1 => Some(FirmwareError::Fetch),
        2 => Some(FirmwareError::NoSlot),
        3 => Some(FirmwareError::Write),
        4 => Some(FirmwareError::InvalidImage),
        5 => Some(FirmwareError::Cancelled),
        6 => Some(FirmwareError::NoMemory),
        _ => None,
    }
}

impl OtaState {
    pub fn start(&self, url: &str) {
        if let Ok(mut held) = self.url.lock() {
            *held = fixed_text_96(url);
        }
        self.downloaded.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        self.error.store(NO_ERROR, Ordering::Relaxed);
        self.phase.store(OtaPhase::Downloading as u8, Ordering::Release);
    }

    pub fn set_total(&self, bytes: u32) {
        self.total.store(bytes, Ordering::Relaxed);
    }

    pub fn add_downloaded(&self, bytes: u32) {
        self.downloaded.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn set_phase(&self, phase: OtaPhase) {
        self.phase.store(phase as u8, Ordering::Release);
    }

    pub fn fail(&self, error: FirmwareError) {
        self.error.store(error_code(error), Ordering::Relaxed);
        self.phase.store(OtaPhase::Failed as u8, Ordering::Release);
    }

    pub fn phase(&self) -> OtaPhase {
        OtaPhase::from_u8(self.phase.load(Ordering::Acquire))
    }

    pub fn snapshot(&self, slot: FirmwareSlot) -> OtaSnapshot {
        OtaSnapshot {
            phase: self.phase(),
            downloaded_bytes: self.downloaded.load(Ordering::Relaxed),
            total_bytes: self.total.load(Ordering::Relaxed),
            error: error_from_code(self.error.load(Ordering::Relaxed)),
            url: self.url.lock().map(|u| u.clone()).unwrap_or_default(),
            slot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot() -> FirmwareSlot {
        FirmwareSlot::new("ota_0", false, true)
    }

    #[test]
    fn a_new_run_clears_the_previous_failure() {
        let state = OtaState::default();
        state.start("http://x/a.bin");
        state.fail(FirmwareError::Fetch);
        assert_eq!(state.snapshot(slot()).error, Some(FirmwareError::Fetch));

        state.start("http://x/b.bin");
        let snap = state.snapshot(slot());
        assert_eq!(snap.error, None, "the new run must not inherit the old error");
        assert_eq!(snap.phase, OtaPhase::Downloading);
        assert_eq!(snap.downloaded_bytes, 0);
        assert_eq!(snap.url.as_str(), "http://x/b.bin");
    }

    #[test]
    fn progress_without_a_total_reports_no_percentage() {
        let state = OtaState::default();
        state.start("http://x/a.bin");
        state.add_downloaded(4096);
        assert_eq!(state.snapshot(slot()).percent(), None);

        state.set_total(8192);
        assert_eq!(state.snapshot(slot()).percent(), Some(50));
    }

    #[test]
    fn a_written_image_refuses_a_second_update_but_a_failure_does_not() {
        assert!(OtaPhase::Idle.accepts_new_update());
        assert!(OtaPhase::Failed.accepts_new_update());
        assert!(!OtaPhase::Downloading.accepts_new_update());
        assert!(!OtaPhase::Finishing.accepts_new_update());
        assert!(!OtaPhase::ReadyToReboot.accepts_new_update());
    }

    #[test]
    fn progress_is_clamped_at_a_hundred() {
        let state = OtaState::default();
        state.start("u");
        state.set_total(1000);
        state.add_downloaded(4000);
        assert_eq!(state.snapshot(slot()).percent(), Some(100));
    }
}
