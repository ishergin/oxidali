use dali2rust_contracts::msg::ClusterRecordKind;
use dali2rust_domain::registry::{
    ClusterLampInfoView, ClusterLampReadPort, ClusterLampStateView, ClusterPartReadPort,
};

use crate::frame::{KelvinRange, LampCapabilities, LampColour, LampState, PartState};
use crate::presence::HALF_CLOCK_MS;

use super::set::{HeldIdentity, HeldLampInfo, LampInfoSet, LampSet, StateSet, LAMP_SLOTS};

pub const XY_WIRE_SCALE: f64 = 65_535.0;

pub const OLDEST_OBSERVATION_AGE_MS: u32 = HALF_CLOCK_MS - 1;

pub const fn observation_age_ms(now_ms: u32, observed_at_ms: u32) -> u32 {
    let age_ms = now_ms.wrapping_sub(observed_at_ms);
    if age_ms > OLDEST_OBSERVATION_AGE_MS {
        OLDEST_OBSERVATION_AGE_MS
    } else {
        age_ms
    }
}

fn xy_on_wire(value: f64) -> u16 {
    let scaled = (value.clamp(0.0, 1.0) * XY_WIRE_SCALE).round();
    scaled as u16
}

fn lamp_state(lamp: u8, view: ClusterLampStateView, now_ms: u32) -> LampState {
    LampState {
        lamp,
        power: view.power,
        level: view.level,
        colour: LampColour {
            mode: view.color_mode,
            kelvin: view.kelvin,
            xy: view.xy.map(|(x, y)| [xy_on_wire(x), xy_on_wire(y)]),
            rgb: view.rgb.map(|(r, g, b)| [r, g, b]),
            waf: view.waf.map(|(w, a, f)| [w, a, f]),
        },
        age_ms: view.observed_at_ms.map(|at_ms| observation_age_ms(now_ms, at_ms)),
        source: view.value_source,
        absent: view.absent,
    }
}

fn lamp_ids(ids: u64) -> impl Iterator<Item = u8> {
    (0_u8..).take(LAMP_SLOTS).filter(move |lamp| ids & 1_u64.checked_shl(u32::from(*lamp)).unwrap_or(0) != 0)
}

pub(crate) fn fill_state(
    set: &mut StateSet,
    (lamps, parts): (&dyn ClusterLampReadPort, &dyn ClusterPartReadPort),
    adapter: u8,
    now_ms: u32,
) {
    set.clear();
    for lamp in lamp_ids(lamps.cluster_lamp_ids(adapter)) {
        if let Some(view) = lamps.cluster_lamp_state(adapter, lamp) {
            set.hold_lamp(lamp_state(lamp, view, now_ms));
        }
    }
    fill_parts(set, (lamps, parts), adapter);
}

fn fill_parts(set: &mut StateSet, (lamps, parts): (&dyn ClusterLampReadPort, &dyn ClusterPartReadPort), adapter: u8) {
    let keys = parts.cluster_part_keys(ClusterRecordKind::Channel).iter().filter(|(part_adapter, _)| *part_adapter == adapter);
    for (_, group) in keys {
        let Some(part) = parts.cluster_part(ClusterRecordKind::Channel, adapter, group) else {
            continue;
        };
        let lit = lamps.cluster_group_lit(adapter, group);
        let (commanded, known, members) = (lit.commanded, lit.known, lit.members);
        set.hold_part(PartState { group, channel: part.record, commanded, lit: lit.lit, known, members });
    }
}

fn held_info(view: ClusterLampInfoView) -> HeldLampInfo {
    let caps = view.capabilities;
    HeldLampInfo {
        name: view.name,
        device_type: view.device_type,
        device_type_overridden: view.device_type_overridden,
        device_types: view.device_types.map_or(0, |types| types.bits()),
        color_mode: view.color_mode,
        color_mode_overridden: view.color_mode_overridden,
        capabilities: LampCapabilities {
            brightness: caps.brightness,
            cct: caps.cct,
            xy: caps.xy,
            rgb: caps.rgb,
            rgbwaf: caps.rgbwaf,
            scenes: caps.scenes,
            groups: caps.groups,
        },
        kelvin_range: view.kelvin_range.map(|range| KelvinRange { min: range.min_kelvin, max: range.max_kelvin }),
        bound: view.bound,
        ha_entity: view.ha_entity,
    }
}

pub(crate) fn fill_info(set: &mut LampInfoSet, lamps: &dyn ClusterLampReadPort, adapter: u8) {
    set.clear();
    let identity = lamps.cluster_ha_identity();
    set.hold_owner(HeldIdentity { controller_id: identity.controller_id, discovery_prefix: identity.discovery_prefix });
    for lamp in lamp_ids(lamps.cluster_lamp_ids(adapter)) {
        if let Some(view) = lamps.cluster_lamp_info(adapter, lamp) {
            set.hold_lamp(lamp, held_info(view));
        }
    }
}
