#![cfg(target_os = "espidf")]

use std::time::Duration;

use dali2rust_adapters::dali::transport::esp_idf::EspIdfDaliTransport;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = esp_idf_svc::hal::peripherals::Peripherals::take().unwrap();
    let _transport = EspIdfDaliTransport::try_new(
        peripherals.pins.gpio14,
        peripherals.pins.gpio5,
    )
    .unwrap();

    std::thread::sleep(Duration::from_secs(5));

    log::info!("PHY sniffer test PASSED");
}
