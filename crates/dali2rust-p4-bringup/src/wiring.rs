use esp_idf_svc::hal::delay::BLOCK;
use esp_idf_svc::hal::gpio::{InputPin, OutputPin};
use esp_idf_svc::hal::i2c::{I2c, I2cConfig, I2cDriver};
use esp_idf_svc::hal::units::Hertz;

use dali2rust_bsp::esp32p4::pins;

const SSD1306_ADDRS: [u8; 2] = [0x3C, 0x3D];

const SCAN_FIRST: u8 = 0x08;
const SCAN_LAST: u8 = 0x77;

const SCAN_HZ: u32 = 100_000;

pub fn scan_i2c<'d>(
    i2c: impl I2c + 'd,
    sda: impl InputPin + OutputPin + 'd,
    scl: impl InputPin + OutputPin + 'd,
) {
    let config = I2cConfig::new().baudrate(Hertz(SCAN_HZ));
    let mut drv = match I2cDriver::new(i2c, sda, scl, &config) {
        Ok(d) => d,
        Err(e) => {
            log::error!("i2c: driver init failed: {e:?}");
            return;
        }
    };

    let found: Vec<u8> = (SCAN_FIRST..=SCAN_LAST)
        .filter(|addr| drv.write(*addr, &[], BLOCK).is_ok())
        .collect();

    report_scan(&found);
}

fn report_scan(found: &[u8]) {
    let (sda, scl) = (pins::I2C_SDA_GPIO, pins::I2C_SCL_GPIO);
    if found.is_empty() {
        log::warn!(
            "i2c: NOTHING answered on sda=GPIO{sda} scl=GPIO{scl} — check both solder \
             joints plus GND and 3V3; the display needs all four"
        );
        return;
    }
    match found.iter().find(|a| SSD1306_ADDRS.contains(a)) {
        Some(a) => log::info!(
            "i2c: SSD1306 found at 0x{a:02x} on sda=GPIO{sda} scl=GPIO{scl} (all: {found:02x?})"
        ),
        None => log::warn!(
            "i2c: {} device(s) on sda=GPIO{sda} scl=GPIO{scl} ({found:02x?}) but none is an \
             SSD1306 (expected 0x3c or 0x3d)",
            found.len()
        ),
    }
}
