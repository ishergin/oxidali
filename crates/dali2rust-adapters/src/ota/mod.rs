pub mod host;

#[cfg(target_os = "espidf")]
pub mod esp_idf;

use std::sync::Arc;

use dali2rust_platform::firmware::{FirmwareImageSource, FirmwareUpdatePort};

#[derive(Clone)]
pub struct FirmwareUpdatePorts {
    pub port: Arc<dyn FirmwareUpdatePort>,
    pub source: Arc<dyn FirmwareImageSource>,
}

impl FirmwareUpdatePorts {
    #[cfg(target_os = "espidf")]
    pub fn esp() -> Self {
        Self {
            port: Arc::new(esp_idf::EspFirmwarePort::default()),
            source: Arc::new(esp_idf::EspImageSource),
        }
    }

    pub fn host() -> Self {
        Self {
            port: Arc::new(host::SimFirmwarePort::default()),
            source: Arc::new(host::HostImageSource),
        }
    }
}
