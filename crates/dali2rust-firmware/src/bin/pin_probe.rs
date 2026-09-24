#![allow(unexpected_cfgs, reason = "ESP-IDF target cfg is external")]

#[cfg(target_os = "espidf")]
use embassy_executor as _;

#[cfg(target_os = "espidf")]
type ProbePins = Vec<(u8, esp_idf_svc::hal::gpio::PinDriver<'static, esp_idf_svc::hal::gpio::Input>)>;

#[cfg(target_os = "espidf")]
fn sample_window(pins: &ProbePins) -> String {
    let mut last: Vec<bool> = pins.iter().map(|(_, d)| d.is_high()).collect();
    let mut edges = vec![0u32; pins.len()];
    for _ in 0..3000 {
        for (i, (_, d)) in pins.iter().enumerate() {
            let lvl = d.is_high();
            if lvl != last[i] {
                edges[i] += 1;
                last[i] = lvl;
            }
        }
        // sleep-ok: 1 kHz diagnostic edge-sampling cadence (HIL bring-up binary)
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let mut report = String::new();
    for (i, (num, d)) in pins.iter().enumerate() {
        report.push_str(&format!(
            "GPIO{num}: edges={} level={} | ",
            edges[i],
            if d.is_high() { "H" } else { "L" }
        ));
    }
    report
}

#[cfg(target_os = "espidf")]
fn main() {
    use esp_idf_svc::hal::gpio::{PinDriver, Pull};

    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let p = esp_idf_svc::hal::peripherals::Peripherals::take().unwrap();
    let pins: ProbePins = vec![
        (54, PinDriver::input(p.pins.gpio54, Pull::Floating).unwrap()),
        (48, PinDriver::input(p.pins.gpio48, Pull::Floating).unwrap()),
        (47, PinDriver::input(p.pins.gpio47, Pull::Floating).unwrap()),
        (46, PinDriver::input(p.pins.gpio46, Pull::Floating).unwrap()),
        (33, PinDriver::input(p.pins.gpio33, Pull::Floating).unwrap()),
        (32, PinDriver::input(p.pins.gpio32, Pull::Floating).unwrap()),
        (27, PinDriver::input(p.pins.gpio27, Pull::Floating).unwrap()),
        (26, PinDriver::input(p.pins.gpio26, Pull::Floating).unwrap()),
        (23, PinDriver::input(p.pins.gpio23, Pull::Floating).unwrap()),
        (22, PinDriver::input(p.pins.gpio22, Pull::Floating).unwrap()),
        (21, PinDriver::input(p.pins.gpio21, Pull::Floating).unwrap()),
        (20, PinDriver::input(p.pins.gpio20, Pull::Floating).unwrap()),
        (19, PinDriver::input(p.pins.gpio19, Pull::Floating).unwrap()),
        (18, PinDriver::input(p.pins.gpio18, Pull::Floating).unwrap()),
        (17, PinDriver::input(p.pins.gpio17, Pull::Floating).unwrap()),
        (16, PinDriver::input(p.pins.gpio16, Pull::Floating).unwrap()),
        (15, PinDriver::input(p.pins.gpio15, Pull::Floating).unwrap()),
        (14, PinDriver::input(p.pins.gpio14, Pull::Floating).unwrap()),
        (8, PinDriver::input(p.pins.gpio8, Pull::Floating).unwrap()),
        (7, PinDriver::input(p.pins.gpio7, Pull::Floating).unwrap()),
        (6, PinDriver::input(p.pins.gpio6, Pull::Floating).unwrap()),
        (5, PinDriver::input(p.pins.gpio5, Pull::Floating).unwrap()),
        (4, PinDriver::input(p.pins.gpio4, Pull::Floating).unwrap()),
        (3, PinDriver::input(p.pins.gpio3, Pull::Floating).unwrap()),
        (2, PinDriver::input(p.pins.gpio2, Pull::Floating).unwrap()),
    ];

    log::info!("pin_probe: sampling {} candidate pins at ~1 kHz", pins.len());
    loop {
        let report = sample_window(&pins);
        log::info!("pin_probe: {report}");
    }
}

#[cfg(not(target_os = "espidf"))]
fn main() {
    eprintln!(
        "This binary targets ESP-IDF. Build with: cargo build --target riscv32imafc-esp-espidf"
    );
    std::process::exit(1);
}
