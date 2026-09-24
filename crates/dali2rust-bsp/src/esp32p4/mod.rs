pub mod emac_dma;
pub mod pins;

#[cfg(target_os = "espidf")]
pub mod eth;
