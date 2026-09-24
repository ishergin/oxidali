use dali2rust_contracts::msg::{ColorMode, PowerState};
use dali2rust_domain::dali::device::MAX_ARC_POWER_LEVEL;
use dali2rust_domain::registry::{
    ColorTemperatureRangeView, HaGroupStateView, HaGroupView, HaLampView, HaPublishReadPort,
    HaSceneView,
};
use dali2rust_platform::small_sort::insertion_sort_by;

use super::groups::{
    group_capabilities, group_members, group_members_applied, group_record, GROUP_COUNT,
    VIRTUAL_LAMP_COUNT,
};
use super::scenes::{scene_record, SCENE_COUNT};
use super::runtime_fields::RuntimeObservationData;
use super::store::{Inner, RegistryStore};

fn group_view(inner: &Inner, adapter_id: u8, group_id: u8) -> Option<HaGroupView> {
    let record = group_record(inner, adapter_id, group_id);
    if !record.ha_entity_enabled || group_members(inner, adapter_id, group_id).is_empty() {
        return None;
    }
    Some(HaGroupView {
        group_id,
        name: record.name.as_str().to_string(),
        ha_entity_enabled: true,
        capabilities: group_capabilities(inner, adapter_id, group_id),
        state: group_state(inner, adapter_id, group_id),
    })
}

fn lamp_view(inner: &Inner, adapter_id: u8, virtual_lamp_id: u8) -> Option<HaLampView> {
    let lamp = inner.lamps.get(&(adapter_id, virtual_lamp_id))?;
    if !lamp.ha_entity_enabled {
        return None;
    }
    let caps = inner.vl_capability_view(adapter_id, virtual_lamp_id);
    let range = caps.binding_short.and_then(|short| {
        inner
            .physical_devices
            .get(&(adapter_id, short))
            .and_then(|pd| {
                ColorTemperatureRangeView::from_mirek(pd.tc_coolest_mirek, pd.tc_warmest_mirek)
            })
    });
    let unreachable = caps.binding_short.is_some_and(|short| {
        inner
            .physical_devices
            .get(&(adapter_id, short))
            .is_some_and(|pd| {
                pd.runtime
                    .error
                    .as_ref()
                    .is_some_and(dali2rust_contracts::msg::CompactErrorPayload::is_device_absent)
            })
    });
    Some(HaLampView {
        virtual_lamp_id,
        name: lamp.name.as_str().to_string(),
        ha_entity_enabled: true,
        capabilities: caps.capabilities,
        color_temperature_range: range,
        unreachable,
    })
}

pub(crate) fn group_state_for_rules(inner: &Inner, adapter_id: u8, group_id: u8) -> (bool, u8) {
    let members = super::groups::group_members_applied(inner, adapter_id, group_id);
    let mut any_on = false;
    for virtual_lamp_id in &members {
        let Some(short) = inner
            .lamps
            .get(&(adapter_id, *virtual_lamp_id))
            .and_then(|lamp| lamp.binding_short)
        else {
            continue;
        };
        if let Some(pd) = inner.physical_devices.get(&(adapter_id, short)) {
            if pd.runtime.power == PowerState::On {
                any_on = true;
                break;
            }
        }
    }
    (any_on, u8::try_from(members.len()).unwrap_or(u8::MAX))
}

fn group_state(inner: &Inner, adapter_id: u8, group_id: u8) -> HaGroupStateView {
    let mut on = 0u32;
    let mut level_known = 0u32;
    let mut level_sum = 0u32;
    let mut colours = ColourAgreement::default();
    for virtual_lamp_id in group_members_applied(inner, adapter_id, group_id) {
        let Some(short) = inner
            .lamps
            .get(&(adapter_id, virtual_lamp_id))
            .and_then(|l| l.binding_short)
        else {
            continue;
        };
        let Some(pd) = inner.physical_devices.get(&(adapter_id, short)) else {
            continue;
        };
        if pd.runtime.power != PowerState::On {
            continue;
        }
        on += 1;
        if let Some(level) = pd.runtime_level {
            level_known += 1;
            level_sum += u32::from(level);
        }
        colours.note(&pd.runtime);
    }
    let (mode, kelvin, xy, rgb) = colours.resolve();
    HaGroupStateView {
        commanded: inner.group_commanded.contains(&(adapter_id, group_id)),
        any_on: on > 0,
        brightness: (level_known > 0)
            .then(|| (level_sum / level_known).min(u32::from(MAX_ARC_POWER_LEVEL)) as u8),
        color_mode: mode,
        color_temperature_kelvin: kelvin,
        rgb,
        xy,
    }
}

#[derive(Default)]
struct ColourAgreement {
    kelvin: Option<Option<u16>>,
    xy: Option<Option<(f64, f64)>>,
    rgb: Option<Option<(u8, u8, u8)>>,
}

impl ColourAgreement {
    fn note(&mut self, runtime: &RuntimeObservationData) {
        match runtime.color_mode {
            ColorMode::Cct => Self::agree(&mut self.kelvin, runtime.kelvin),
            ColorMode::Xy => Self::agree(&mut self.xy, runtime.xy),
            ColorMode::Rgb | ColorMode::Rgbwaf => Self::agree(&mut self.rgb, runtime.rgb),
            _ => {}
        }
    }

    fn agree<T: PartialEq + Copy>(slot: &mut Option<Option<T>>, seen: Option<T>) {
        *slot = Some(match *slot {
            None => seen,
            Some(prior) if prior == seen => prior,
            Some(_) => None,
        });
    }

    #[allow(clippy::type_complexity, reason = "one tuple, consumed on the next line")]
    fn resolve(
        self,
    ) -> (
        Option<ColorMode>,
        Option<u16>,
        Option<(f64, f64)>,
        Option<(u8, u8, u8)>,
    ) {
        if let Some(triple) = self.rgb.flatten() {
            (Some(ColorMode::Rgb), None, None, Some(triple))
        } else if let Some(pair) = self.xy.flatten() {
            (Some(ColorMode::Xy), None, Some(pair), None)
        } else if let Some(k) = self.kelvin.flatten() {
            (Some(ColorMode::Cct), Some(k), None, None)
        } else {
            (None, None, None, None)
        }
    }
}

impl HaPublishReadPort for RegistryStore {
    fn ha_input_view(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<dali2rust_domain::registry::HaInputView> {
        self.ha_input_views(adapter_id)
            .into_iter()
            .find(|view| view.short_address == short_address)
    }

    fn ha_input_views(&self, adapter_id: u8) -> Vec<dali2rust_domain::registry::HaInputView> {
        let g = self.read_inner();
        let mut out: Vec<_> = g
            .input_devices
            .values()
            .filter(|record| record.adapter_id == adapter_id)
            .map(|record| dali2rust_domain::registry::HaInputView {
                short_address: record.short_address,
                name: record.name.clone(),
                ha_expose: record.ha_expose,
                instances: record
                    .instances
                    .iter()
                    .map(|i| dali2rust_domain::registry::HaInputInstanceView {
                        instance_number: i.instance_number,
                        instance_type: i.instance_type,
                    })
                    .collect(),
            })
            .collect();
        insertion_sort_by(&mut out, |a, b| a.short_address > b.short_address);
        out
    }

    fn ha_lamp_views(&self, adapter_id: u8) -> Vec<HaLampView> {
        let g = self.read_inner();
        let mut ids: Vec<u8> = g
            .lamps
            .keys()
            .filter(|(a, _)| *a == adapter_id)
            .map(|(_, vlid)| *vlid)
            .collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| lamp_view(&g, adapter_id, id))
            .collect()
    }

    fn ha_lamp_view(&self, adapter_id: u8, virtual_lamp_id: u8) -> Option<HaLampView> {
        lamp_view(&self.read_inner(), adapter_id, virtual_lamp_id)
    }

    fn ha_lamp_ids_for_short(&self, adapter_id: u8, short_address: u8) -> Vec<u8> {
        let g = self.read_inner();
        let mut ids: Vec<u8> = g
            .lamps
            .iter()
            .filter(|((a, _), lamp)| {
                *a == adapter_id && lamp.binding_short == Some(short_address)
            })
            .map(|((_, vlid), _)| *vlid)
            .collect();
        ids.sort_unstable();
        ids
    }

    fn ha_group_views(&self, adapter_id: u8) -> Vec<HaGroupView> {
        let g = self.read_inner();
        (0..GROUP_COUNT)
            .filter_map(|group_id| group_view(&g, adapter_id, group_id))
            .collect()
    }

    fn ha_group_view(&self, adapter_id: u8, group_id: u8) -> Option<HaGroupView> {
        group_view(&self.read_inner(), adapter_id, group_id)
    }

    fn ha_retracted_lamp_ids(&self, adapter_id: u8) -> Vec<u8> {
        let g = self.read_inner();
        (0..VIRTUAL_LAMP_COUNT)
            .filter(|id| lamp_view(&g, adapter_id, *id).is_none())
            .collect()
    }

    fn ha_retracted_group_ids(&self, adapter_id: u8) -> Vec<u8> {
        let exposed: Vec<u8> = self
            .ha_group_views(adapter_id)
            .into_iter()
            .map(|v| v.group_id)
            .collect();
        (0..GROUP_COUNT).filter(|id| !exposed.contains(id)).collect()
    }

    fn ha_active_scene(&self, adapter_id: u8) -> Option<u8> {
        self.read_inner().active_scene.get(&adapter_id).copied()
    }

    fn ha_scene_views(&self, adapter_id: u8) -> Vec<HaSceneView> {
        let g = self.read_inner();
        (0..SCENE_COUNT)
            .filter_map(|scene_id| {
                let record = scene_record(&g, adapter_id, scene_id);
                record.ha_select_enabled.then(|| HaSceneView {
                    scene_id,
                    name: record.name.as_str().to_string(),
                    ha_select_enabled: true,
                })
            })
            .collect()
    }
}
