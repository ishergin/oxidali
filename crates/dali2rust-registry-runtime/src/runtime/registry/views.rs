use dali2rust_contracts::msg::{ColorMode, DeviceType};
use dali2rust_domain::registry::{
    seed_capability_from_color_mode, CapabilityFlagsView, ColorTemperatureRangeView,
    PhysicalDeviceStateView, RuntimeErrorView,
    VirtualLampView,
};

use super::conversions::{
    color_mode_str, device_type_str, effective_color_mode, failure_status_view,
    last_dapc_source_view, observation_source_from_runtime, override_source_str, power_state_str,
    status_flags_view, unbound_capabilities_view,
};
use super::physical_devices::PhysicalDeviceRecord;
use super::virtual_lamps::VlRecord;

pub(crate) fn unbound_state_view() -> PhysicalDeviceStateView {
    PhysicalDeviceStateView {
        power: "unknown".to_string(),
        color_mode: "unknown".to_string(),
        ..Default::default()
    }
}

pub(crate) fn physical_device_state_view(
    rec: &PhysicalDeviceRecord,
    color_mode_fallback: ColorMode,
) -> PhysicalDeviceStateView {
    let cm_display = effective_color_mode(
        rec.runtime.color_mode,
        rec.runtime.has_observation,
        color_mode_fallback,
    );
    PhysicalDeviceStateView {
        power: power_state_str(rec.runtime.power, rec.runtime.has_observation).to_string(),
        level: rec.runtime_level,
        color_mode: color_mode_str(cm_display).to_string(),
        color_temperature_kelvin: rec.runtime.kelvin,
        xy: rec.runtime.xy,
        rgb: rec.runtime.rgb,
        waf: rec.runtime.waf,
        status: status_flags_view(rec.runtime.status_flags.as_ref()),
        failure_status: failure_status_view(rec.runtime.failure_status.as_ref()),
        value_source: rec
            .runtime
            .value_source
            .map(observation_source_from_runtime),
        last_seen_ms: rec.runtime.last_seen_ms,
        last_dapc_source: last_dapc_source_view(rec.runtime.last_dapc_source),
        error: rec
            .runtime
            .error
            .as_ref()
            .map(|e| RuntimeErrorView { code: e.code }),
    }
}

pub(crate) fn effective_capabilities_view(
    rec: &PhysicalDeviceRecord,
    color_mode_effective: ColorMode,
) -> CapabilityFlagsView {
    let mut caps = rec.capability_flags();
    seed_capability_from_color_mode(&mut caps, color_mode_effective);
    caps
}

pub(crate) fn bound_state_and_capabilities(
    pr: &PhysicalDeviceRecord,
    cm_fallback: ColorMode,
) -> (PhysicalDeviceStateView, CapabilityFlagsView) {
    (
        physical_device_state_view(pr, cm_fallback),
        effective_capabilities_view(pr, cm_fallback),
    )
}

pub(crate) fn resolve_effective_device_type(
    physical_rec: Option<&PhysicalDeviceRecord>,
) -> (DeviceType, &'static str) {
    physical_rec.map_or((DeviceType::Unknown, override_source_str(false)), |p| {
        (
            p.effective_device_type(),
            override_source_str(p.device_type_override.is_some()),
        )
    })
}

pub(crate) fn resolve_effective_color_mode(
    physical_rec: Option<&PhysicalDeviceRecord>,
) -> (ColorMode, &'static str) {
    physical_rec.map_or((ColorMode::Unknown, override_source_str(false)), |p| {
        (
            p.effective_color_mode(),
            override_source_str(p.color_mode_override.is_some()),
        )
    })
}

pub(crate) fn virtual_lamp_view_from_parts(
    adapter_id: u8,
    virtual_lamp_id: u8,
    vl: &VlRecord,
    physical_rec: Option<&PhysicalDeviceRecord>,
) -> VirtualLampView {
    let (dt_eff, dt_src) = resolve_effective_device_type(physical_rec);
    let (cm_eff, cm_src) = resolve_effective_color_mode(physical_rec);
    let binding_sa = vl.binding_short;
    let (state, capabilities) = physical_rec
        .map(|pr| bound_state_and_capabilities(pr, cm_eff))
        .unwrap_or_else(|| (unbound_state_view(), unbound_capabilities_view()));

    VirtualLampView {
        adapter_id,
        virtual_lamp_id,
        name: vl.name.as_str().to_string(),
        device_type_effective: device_type_str(dt_eff).to_string(),
        device_type_source: dt_src.to_string(),
        color_mode_effective: color_mode_str(cm_eff).to_string(),
        color_mode_source: cm_src.to_string(),
        binding_short: binding_sa,
        ha_entity_enabled: vl.ha_entity_enabled,
        state,
        capabilities,
        color_temperature_range: physical_rec.and_then(|pr| {
            ColorTemperatureRangeView::from_mirek(pr.tc_coolest_mirek, pr.tc_warmest_mirek)
        }),
    }
}

pub(crate) fn apply_dt8_capability_flags(
    rec: &mut PhysicalDeviceRecord,
    dt8_tc_capable: bool,
    dt8_xy_capable: bool,
    dt8_rgb_capable: bool,
    dt8_rgbwaf_capable: bool,
    color_mode: ColorMode,
) -> bool {
    let prev = rec.capability_flags();
    let mut caps = prev;
    caps.brightness = true;
    caps.cct |= dt8_tc_capable;
    caps.xy |= dt8_xy_capable;
    caps.rgb |= dt8_rgb_capable;
    caps.rgbwaf |= dt8_rgbwaf_capable;
    seed_capability_from_color_mode(&mut caps, color_mode);
    let changed = caps != prev;
    rec.set_capability_flags(caps);
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dt8_capabilities_are_sticky_across_a_lost_features_read() {
        let mut rec = PhysicalDeviceRecord::empty(1);

        apply_dt8_capability_flags(&mut rec, true, false, true, false, ColorMode::Rgb);
        assert!(rec.cap_cct && rec.cap_rgb && !rec.cap_xy);

        apply_dt8_capability_flags(&mut rec, false, false, false, false, ColorMode::Rgb);
        assert!(rec.cap_cct, "cct capability must survive a lost features read");
        assert!(rec.cap_rgb);
    }

    #[test]
    fn active_color_mode_seeds_capability_without_features() {
        let mut rec = PhysicalDeviceRecord::empty(2);
        apply_dt8_capability_flags(&mut rec, false, false, false, false, ColorMode::Cct);
        assert!(rec.cap_cct && !rec.cap_rgb && !rec.cap_xy);
    }
}
