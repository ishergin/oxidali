use dali2rust_contracts::msg::{fixed_text_32, PowerState};
use dali2rust_domain::registry::{
    known_device_types, ClusterGroupLitView, ClusterHaIdentityView, ClusterLampInfoView,
    ClusterLampReadPort, ClusterLampStateView, ColorTemperatureRangeView,
};

use super::conversions::{effective_color_mode, unbound_capabilities_view};
use super::groups::{applied_member_devices, VIRTUAL_LAMP_COUNT};
use super::physical_devices::PhysicalDeviceRecord;
use super::store::{Inner, RegistryStore};
use super::views::{effective_capabilities_view, resolve_effective_color_mode, resolve_effective_device_type};
use super::virtual_lamps::VlRecord;

fn bound_record<'a>(inner: &'a Inner, adapter_id: u8, vl: &VlRecord) -> Option<&'a PhysicalDeviceRecord> {
    let short = vl.binding_short?;
    inner.physical_devices.get(&(adapter_id, short)).map(|boxed| boxed.as_ref())
}

fn lamp_state(rec: &PhysicalDeviceRecord) -> ClusterLampStateView {
    let observed = rec.runtime.has_observation;
    let fallback = rec.effective_color_mode();
    ClusterLampStateView {
        power: if observed { rec.runtime.power } else { PowerState::Unknown },
        level: rec.runtime_level,
        color_mode: effective_color_mode(rec.runtime.color_mode, observed, fallback),
        kelvin: rec.runtime.kelvin,
        xy: rec.runtime.xy,
        rgb: rec.runtime.rgb,
        waf: rec.runtime.waf,
        observed_at_ms: rec.runtime.observed_at_mono_ms,
        value_source: rec.runtime.value_source,
        absent: rec.runtime.reports_absence(),
    }
}

const UNBOUND_STATE: ClusterLampStateView = ClusterLampStateView {
    power: PowerState::Unknown,
    level: None,
    color_mode: dali2rust_contracts::msg::ColorMode::Unknown,
    kelvin: None,
    xy: None,
    rgb: None,
    waf: None,
    observed_at_ms: None,
    value_source: None,
    absent: false,
};

fn lamp_info(vl: &VlRecord, rec: Option<&PhysicalDeviceRecord>) -> ClusterLampInfoView {
    let (device_type, _) = resolve_effective_device_type(rec);
    let (color_mode, _) = resolve_effective_color_mode(rec);
    ClusterLampInfoView {
        name: vl.name.clone(),
        device_type,
        device_type_overridden: rec.is_some_and(|rec| rec.device_type_override.is_some()),
        device_types: rec.and_then(|rec| known_device_types(rec.supported_device_types, rec.device_type_override)),
        color_mode,
        color_mode_overridden: rec.is_some_and(|rec| rec.color_mode_override.is_some()),
        capabilities: rec.map_or_else(unbound_capabilities_view, |rec| effective_capabilities_view(rec, color_mode)),
        kelvin_range: rec.and_then(|rec| ColorTemperatureRangeView::from_mirek(rec.tc_coolest_mirek, rec.tc_warmest_mirek)),
        bound: vl.binding_short.is_some(),
        ha_entity: vl.ha_entity_enabled,
    }
}

fn group_lit(inner: &Inner, adapter_id: u8, group_id: u8) -> ClusterGroupLitView {
    let mut view = ClusterGroupLitView {
        commanded: inner.group_commanded.contains(&(adapter_id, group_id)),
        ..ClusterGroupLitView::default()
    };
    for device in applied_member_devices(inner, adapter_id, group_id) {
        view.members = view.members.saturating_add(1);
        let power = device.map_or(PowerState::Unknown, |rec| lamp_state(rec).power);
        let answering = device.is_some_and(|rec| !rec.runtime.reports_absence());
        if power != PowerState::Unknown && answering {
            view.known = view.known.saturating_add(1);
            view.lit = view.lit.saturating_add(u8::from(power == PowerState::On));
        }
    }
    view
}

impl ClusterLampReadPort for RegistryStore {
    fn cluster_ha_identity(&self) -> ClusterHaIdentityView {
        let inner = self.read_inner();
        let settings = &inner.home_assistant_settings;
        ClusterHaIdentityView {
            controller_id: fixed_text_32(&settings.controller_id),
            discovery_prefix: fixed_text_32(&settings.discovery_prefix),
        }
    }

    fn cluster_lamp_ids(&self, adapter_id: u8) -> u64 {
        let inner = self.read_inner();
        (0..VIRTUAL_LAMP_COUNT)
            .filter(|lamp| inner.lamps.contains_key(&(adapter_id, *lamp)))
            .fold(0, |mask, lamp| mask | (1u64 << lamp))
    }

    fn cluster_lamp_state(&self, adapter_id: u8, lamp: u8) -> Option<ClusterLampStateView> {
        let inner = self.read_inner();
        let vl = inner.lamps.get(&(adapter_id, lamp))?;
        Some(bound_record(&inner, adapter_id, vl).map_or(UNBOUND_STATE, lamp_state))
    }

    fn cluster_lamp_info(&self, adapter_id: u8, lamp: u8) -> Option<ClusterLampInfoView> {
        let inner = self.read_inner();
        let vl = inner.lamps.get(&(adapter_id, lamp))?;
        Some(lamp_info(vl, bound_record(&inner, adapter_id, vl)))
    }

    fn cluster_group_lit(&self, adapter_id: u8, group_id: u8) -> ClusterGroupLitView {
        group_lit(&self.read_inner(), adapter_id, group_id)
    }
}
