use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareError {
    BadUrl,
    Fetch,
    NoSlot,
    Write,
    InvalidImage,
    Cancelled,
    NoMemory,
}

impl fmt::Display for FirmwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FirmwareError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BadUrl => "bad_url",
            Self::Fetch => "fetch_failed",
            Self::NoSlot => "no_ota_slot",
            Self::Write => "write_failed",
            Self::InvalidImage => "invalid_image",
            Self::Cancelled => "cancelled",
            Self::NoMemory => "no_memory",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirmwareSlot {
    pub label: [u8; Self::LABEL_BYTES],
    pub label_len: u8,
    pub pending_verify: bool,
    pub ota_capable: bool,
}

impl FirmwareSlot {
    pub const LABEL_BYTES: usize = 16;

    pub fn new(label: &str, pending_verify: bool, ota_capable: bool) -> Self {
        let mut bytes = [0u8; Self::LABEL_BYTES];
        let src = label.as_bytes();
        let len = src.len().min(Self::LABEL_BYTES);
        bytes[..len].copy_from_slice(&src[..len]);
        Self {
            label: bytes,
            label_len: len as u8,
            pending_verify,
            ota_capable,
        }
    }

    pub fn label_str(&self) -> &str {
        core::str::from_utf8(&self.label[..self.label_len as usize]).unwrap_or("")
    }
}

pub trait FirmwareSink {
    fn total(&mut self, bytes: Option<u32>) -> Result<(), FirmwareError>;
    fn chunk(&mut self, data: &[u8]) -> Result<(), FirmwareError>;
}

pub trait FirmwareImageSource: Send + Sync {
    fn fetch(&self, url: &str, sink: &mut dyn FirmwareSink) -> Result<u32, FirmwareError>;
}

#[derive(Debug, Default)]
pub struct MaintenanceHold(core::sync::atomic::AtomicBool);

impl MaintenanceHold {
    pub const fn new() -> Self {
        Self(core::sync::atomic::AtomicBool::new(false))
    }

    pub fn engage(&self) {
        self.0.store(true, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn release(&self) {
        self.0.store(false, core::sync::atomic::Ordering::Relaxed);
    }

    pub fn engaged(&self) -> bool {
        self.0.load(core::sync::atomic::Ordering::Relaxed)
    }
}

pub trait FirmwareUpdatePort: Send + Sync {
    fn slot(&self) -> FirmwareSlot;

    fn begin(&self, total_bytes: Option<u32>) -> Result<(), FirmwareError>;

    fn write(&self, chunk: &[u8]) -> Result<(), FirmwareError>;

    fn finish(&self) -> Result<(), FirmwareError>;

    fn abort(&self);

    fn mark_valid(&self) -> Result<(), FirmwareError>;

    fn reboot(&self);
}
