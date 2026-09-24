#![cfg(target_os = "espidf")]

use dali2rust_adapters::dali::transport::esp_idf::EspIdfDaliTransport;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = esp_idf_svc::hal::peripherals::Peripherals::take().unwrap();
    let mut _transport = EspIdfDaliTransport::try_new(
        peripherals.pins.gpio14,
        peripherals.pins.gpio5,
    )
    .unwrap();

    log::info!("PHY collision test PASSED");
}
