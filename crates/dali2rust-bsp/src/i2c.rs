#[cfg(target_os = "espidf")]
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver};
#[cfg(target_os = "espidf")]
use esp_idf_svc::hal::units::Hertz;
#[cfg(target_os = "espidf")]
use esp_idf_svc::sys::EspError;

#[cfg(target_os = "espidf")]
pub fn init_i2c<'d>(
    i2c: impl esp_idf_svc::hal::i2c::I2c + 'd,
    sda: impl esp_idf_svc::hal::gpio::InputPin + esp_idf_svc::hal::gpio::OutputPin + 'd,
    scl: impl esp_idf_svc::hal::gpio::InputPin + esp_idf_svc::hal::gpio::OutputPin + 'd,
) -> Result<I2cDriver<'d>, EspError> {
    let config = I2cConfig::new().baudrate(Hertz(400_000));
    I2cDriver::new(i2c, sda, scl, &config)
}

#[cfg(test)]
mod tests {
    #[test]
    fn i2c_module_compiles() {}
}
