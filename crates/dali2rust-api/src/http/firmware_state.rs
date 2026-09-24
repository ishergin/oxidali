use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct FirmwareStateDto {
    pub running_slot: String,
    pub ota_capable: bool,
    pub pending_verify: bool,
    pub update: FirmwareUpdateDto,
}

#[derive(Clone, Debug, Serialize)]
pub struct FirmwareUpdateDto {
    pub state: &'static str,
    pub url: String,
    pub downloaded_bytes: u32,
    pub total_bytes: u32,
    pub percent: Option<u8>,
    pub error: Option<&'static str>,
}

pub trait FirmwareHttpState: Send + Sync {
    fn firmware_dto(&self) -> FirmwareStateDto;
}
