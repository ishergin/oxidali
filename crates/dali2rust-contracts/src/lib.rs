pub mod bus;
pub mod msg;
pub mod source_id;

pub use source_id::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};

#[cfg(test)]
mod ha_stage2_contract_tests;
