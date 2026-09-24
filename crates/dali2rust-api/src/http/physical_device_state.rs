mod attributes;
mod banks;
mod device_core;
mod ports;
mod runtime_state;
mod summary;

#[cfg(test)]
mod tests;

pub use self::attributes::{
    AttributeSectionDto, BusUnitConfigurationDto, Common102AttributesDto, ConditionDto,
    Dt6LedAttributesDto, Dt8ColorAttributesDto, EnergyBankDto, ExtendedAttributesDto,
    GearDiagnosticsDto, GroupsAttributesDto, LuminaireMaintenanceDto, MemoryBusUnitAttributesDto,
    MemoryDiagnosticsAttributesDto, MemoryEnergyAttributesDto, MemoryIdentityAttributesDto,
    MemoryLuminaireAttributesDto, MemoryProfileAttributesDto, ObservedValueDto,
    ScenesAttributesDto, SourceDiagnosticsDto,
};
pub use self::attributes::write_attribute_section;
pub use self::banks::{MemoryBankRangeDto, MemoryBankSummaryDto};
pub use self::device_core::{ColorTemperatureRangeDto, PhysicalDeviceCoreDto};
pub use self::ports::{
    PhysicalDeviceHttpState, PhysicalDeviceHttpStateBridge, PhysicalDevicePatchWatch,
    PhysicalDevicePatchWatchBridge,
};
pub use self::runtime_state::{
    CapabilityFlagsDto, FailureStatusDto, PhysicalDeviceStateDto, RgbDto, RuntimeErrorDto,
    StatusFlagsDto, WafDto, XyDto, XY_FULL_SCALE,
};
pub use self::summary::PhysicalDeviceSummaryDto;

pub(crate) use self::banks::memory_bank_view_to_dto;
pub(crate) use self::device_core::core_view_to_dto;
pub(crate) use self::ports::impl_physical_device_http_state;
pub(crate) use self::runtime_state::{capabilities_view_to_dto, state_view_to_dto};
pub(crate) use self::summary::summary_view_to_dto;
