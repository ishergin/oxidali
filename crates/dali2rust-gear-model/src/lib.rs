pub mod fleet;
pub mod gear;
pub mod luminaire;
pub mod metering;
pub mod rng;
pub mod input_device;

#[cfg(test)]
mod tests;

pub use fleet::{
    bench_fleet, FleetStats, GearFleet, DEFAULT_RESERVED_SHORT_ADDRESSES, RANDOM_ADDRESS_READY_MS,
};
pub use gear::{
    ColorMode, FaultInjection, Gear, GearSpec, DEFAULT_RGBWAF_CHANNELS, DEVICE_TYPE_DIAGNOSTICS,
    DEVICE_TYPE_ENERGY, DEVICE_TYPE_LUMINAIRE_INFO, SCENE_COUNT,
};
pub use metering::MeteringBanks;
pub use rng::Rng;

#[cfg(test)]
mod emulator_import_contract {
    #[allow(unused_imports, reason = "the imports ARE the test — resolution is the assertion")]
    mod exact_emulator_surface {
        pub use crate::{
            bench_fleet, FleetStats, Gear, GearFleet, GearSpec, DEFAULT_RESERVED_SHORT_ADDRESSES,
            DEFAULT_RGBWAF_CHANNELS, DEVICE_TYPE_DIAGNOSTICS, DEVICE_TYPE_ENERGY,
            DEVICE_TYPE_LUMINAIRE_INFO, SCENE_COUNT,
        };
    }
}
