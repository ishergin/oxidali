#[cfg(target_os = "espidf")]
pub mod esp_idf;
pub mod mock;

#[cfg(target_os = "espidf")]
pub use esp_idf::EspMqttBridgeClient;
pub use mock::{MockMqttClient, MockMqttHandle, PublishedMessage};
