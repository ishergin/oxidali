use dali2rust_contracts::msg::{ColorMode, DeviceType, LastDapcSource, RuntimeSource};
use dali2rust_domain::registry::{
    CapabilityFlagsView, FailureStatusView, LastDapcSourceView, ObservationSource, StatusFlagsView,
};

use super::runtime_fields::{FailureStatusData, StatusFlagsData};

pub(crate) fn device_type_str(dt: DeviceType) -> &'static str {
    match dt {
        DeviceType::Dt8Color => "dt8_color",
        DeviceType::Dt6Led => "dt6_led",
        DeviceType::Unknown => "unknown",
        _ => "unknown",
    }
}

pub(crate) fn color_mode_str(cm: ColorMode) -> &'static str {
    cm.rest_name()
}

pub(crate) fn observation_source_from_runtime(src: RuntimeSource) -> ObservationSource {
    match src {
        RuntimeSource::Api => ObservationSource::Api,
        RuntimeSource::Sniffer => ObservationSource::Sniffer,
        RuntimeSource::Poller => ObservationSource::Poller,
        RuntimeSource::Mqtt => ObservationSource::Mqtt,
        RuntimeSource::Hcl => ObservationSource::Hcl,
        RuntimeSource::Cluster => ObservationSource::Cluster,
        RuntimeSource::AdapterProxy => ObservationSource::AdapterProxy,
        RuntimeSource::Rules => ObservationSource::Rules,
        RuntimeSource::Readback => ObservationSource::Api,
    }
}

pub(crate) fn last_dapc_source_view(src: LastDapcSource) -> LastDapcSourceView {
    match src {
        LastDapcSource::Unknown => LastDapcSourceView::Unknown,
        LastDapcSource::Sniffer => LastDapcSourceView::Sniffer,
        LastDapcSource::Scene => LastDapcSourceView::Scene,
        LastDapcSource::Group => LastDapcSourceView::Group,
    }
}

pub(crate) fn status_flags_view(data: Option<&StatusFlagsData>) -> Option<StatusFlagsView> {
    data.map(|sf| StatusFlagsView {
        raw: sf.raw,
        lamp_failure: sf.lamp_failure,
        gear_failure: sf.gear_failure,
        lamp_on: sf.lamp_on,
        limit_error: sf.limit_error,
        fade_running: sf.fade_running,
        reset_state: sf.reset_state,
        missing_short_address: sf.missing_short_address,
        power_cycle_seen: sf.power_cycle_seen,
    })
}

pub(crate) fn failure_status_view(data: Option<&FailureStatusData>) -> Option<FailureStatusView> {
    data.map(|fs| FailureStatusView {
        raw: fs.raw,
        lamp_failure: fs.lamp_failure,
        gear_failure: fs.gear_failure,
        communication_failure: fs.communication_failure,
        source: observation_source_from_runtime(fs.source),
    })
}

pub(crate) fn capabilities_view(
    brightness: bool,
    cct: bool,
    xy: bool,
    rgb: bool,
    rgbwaf: bool,
) -> CapabilityFlagsView {
    CapabilityFlagsView {
        brightness,
        cct,
        xy,
        rgb,
        rgbwaf,
        scenes: true,
        groups: true,
    }
}

pub(crate) fn unbound_capabilities_view() -> CapabilityFlagsView {
    CapabilityFlagsView::default()
}

pub(crate) fn power_state_str(
    power: dali2rust_contracts::msg::PowerState,
    has_observation: bool,
) -> &'static str {
    if !has_observation {
        return "unknown";
    }
    power.rest_name()
}

pub(crate) fn override_source_str(manual_override: bool) -> &'static str {
    if manual_override {
        "manual_override"
    } else {
        "discovered"
    }
}

pub(crate) fn effective_color_mode(
    runtime_cm: ColorMode,
    has_observation: bool,
    fallback_cm: ColorMode,
) -> ColorMode {
    if !has_observation {
        return fallback_cm;
    }
    match runtime_cm {
        ColorMode::None | ColorMode::Unknown => fallback_cm,
        other => other,
    }
}
