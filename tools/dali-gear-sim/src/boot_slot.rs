use core::ffi::CStr;

use esp_idf_svc::sys::{
    esp_ota_get_running_partition, esp_ota_get_state_partition, esp_ota_img_states_t,
    esp_ota_img_states_t_ESP_OTA_IMG_ABORTED, esp_ota_img_states_t_ESP_OTA_IMG_INVALID,
    esp_ota_img_states_t_ESP_OTA_IMG_NEW, esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY,
    esp_ota_img_states_t_ESP_OTA_IMG_VALID, ESP_ERR_NOT_FOUND, ESP_ERR_NOT_SUPPORTED, ESP_OK,
};

pub struct BootSlot {
    pub label: String,
    pub state: &'static str,
}

pub fn read() -> BootSlot {
    // SAFETY: a read of the partition table the bootloader already validated; this image never writes flash.
    let running = unsafe { esp_ota_get_running_partition() };
    if running.is_null() {
        return BootSlot {
            label: "unknown".into(),
            state: "undefined",
        };
    }
    // SAFETY: `running` points at a partition-table entry that lives as long as the program.
    let label = unsafe { CStr::from_ptr((*running).label.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    let mut raw: esp_ota_img_states_t = 0;
    // SAFETY: `running` is valid and `raw` is a live out-parameter.
    let err = unsafe { esp_ota_get_state_partition(running, &mut raw) };
    BootSlot {
        label,
        state: state_token(err, raw),
    }
}

fn state_token(err: i32, raw: esp_ota_img_states_t) -> &'static str {
    if err == ESP_ERR_NOT_FOUND || err == ESP_ERR_NOT_SUPPORTED {
        return "none";
    }
    if err != ESP_OK {
        return "undefined";
    }
    STATE_TOKENS
        .iter()
        .find(|(state, _)| *state == raw)
        .map_or("undefined", |(_, token)| token)
}

const STATE_TOKENS: [(esp_ota_img_states_t, &str); 5] = [
    (esp_ota_img_states_t_ESP_OTA_IMG_NEW, "new"),
    (esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY, "pending_verify"),
    (esp_ota_img_states_t_ESP_OTA_IMG_VALID, "valid"),
    (esp_ota_img_states_t_ESP_OTA_IMG_INVALID, "invalid"),
    (esp_ota_img_states_t_ESP_OTA_IMG_ABORTED, "aborted"),
];
