use crate::runtime::registry::conversions::color_mode_str;
use crate::runtime::registry::physical_devices::{
    forget_scene_level, write_scene_level_read, write_scene_level_written, SceneColourEvidence,
};
use crate::runtime::registry::store::{registry_unix_ms, Inner, RegistryStore};
use dali2rust_contracts::msg::{
    fixed_text_64, ColorMode, ColorValue, DaliSceneTargetState, DaliTargetScope, FixedText64,
    PowerState, SceneMatrixDesiredRow,
};
use dali2rust_domain::dali::devices::dt8_color::{
    dim_level_to_srgb_channel, srgb_channel_to_dim_level, COLOUR_TYPE_BYTE_RGBWAF as SCENE_COLOUR_TYPE_RGBWAF,
    COLOUR_TYPE_BYTE_TC as SCENE_COLOUR_TYPE_TC, COLOUR_TYPE_BYTE_XY as SCENE_COLOUR_TYPE_XY,
};
use dali2rust_domain::dali::device::SCENE_NOT_SET;
use dali2rust_domain::registry::{
    SceneApplyRowView, SceneApplySnapshot, SceneMatrixRowView, SceneMatrixView, SceneReadPort,
    SceneRowStateView, SceneView,
};

pub(crate) use dali2rust_domain::registry::{SCENE_COUNT, VIRTUAL_LAMP_COUNT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ForeignSceneWrite {
    pub scope: DaliTargetScope,
    pub short_address: Option<u8>,
    pub group_id: Option<u8>,
    pub scene_id: u8,
    pub removal: bool,
}

fn foreign_write_reaches(write: &ForeignSceneWrite, short: u8, membership: Option<u16>) -> Option<bool> {
    match write.scope {
        DaliTargetScope::Short => (write.short_address == Some(short)).then_some(true),
        DaliTargetScope::Group => {
            let bit = 1u16.checked_shl(u32::from(write.group_id?))?;
            match membership {
                Some(mask) => (mask & bit != 0).then_some(true),
                None => Some(false),
            }
        }
        DaliTargetScope::Broadcast => Some(true),
        DaliTargetScope::VirtualLamp | DaliTargetScope::AddressRange => None,
    }
}

fn foreign_write_targets(inner: &Inner, adapter_id: u8, write: &ForeignSceneWrite) -> Vec<(u8, bool)> {
    let mut targets: Vec<(u8, bool)> = inner
        .physical_devices
        .iter()
        .filter(|((aid, _), _)| *aid == adapter_id)
        .filter_map(|((_, short), record)| {
            let membership = record.attributes.groups.membership.as_ref().map(|m| m.value);
            foreign_write_reaches(write, *short, membership).map(|certain| (*short, certain))
        })
        .collect();
    targets.sort_unstable();
    targets
}

#[derive(Clone, Debug)]
pub(crate) struct SceneRecord {
    pub name: FixedText64,
    pub ha_select_enabled: bool,
}

impl Default for SceneRecord {
    fn default() -> Self {
        Self {
            name: FixedText64::new(),
            ha_select_enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SceneDesiredRowRecord {
    pub included: bool,
    pub target: Option<DaliSceneTargetState>,
    pub desired_seeded: bool,
}

fn power_str(power: PowerState) -> &'static str {
    match power {
        PowerState::On => "on",
        PowerState::Off => "off",
        _ => "unknown",
    }
}

fn state_from_target(included: bool, target: Option<&DaliSceneTargetState>) -> SceneRowStateView {
    let Some(t) = target.filter(|_| included) else {
        return SceneRowStateView {
            included,
            ..Default::default()
        };
    };
    let color = t.color.as_ref();
    SceneRowStateView {
        included: true,
        power: t.power.map(|p| power_str(p).to_string()),
        level: t.level,
        color_mode: color.map(|c| color_mode_str(c.mode).to_string()),
        color_temperature_kelvin: color.and_then(ColorValue::cct),
        xy: color.and_then(ColorValue::xy_wire),
        rgb: color.and_then(ColorValue::rgb_channels),
        waf: color.and_then(ColorValue::waf_channels),
    }
}

fn binding_short(inner: &Inner, adapter_id: u8, virtual_lamp_id: u8) -> Option<u8> {
    inner
        .lamps
        .get(&(adapter_id, virtual_lamp_id))
        .and_then(|lamp| lamp.binding_short)
}

fn scene_level_evidence(
    inner: &Inner,
    adapter_id: u8,
    scene_id: u8,
    virtual_lamp_id: u8,
) -> Option<Option<u8>> {
    let short = binding_short(inner, adapter_id, virtual_lamp_id)?;
    let device = inner.physical_devices.get(&(adapter_id, short))?;
    let observed = device
        .attributes
        .scenes
        .levels
        .get(scene_id as usize)?
        .as_ref()?;
    Some((observed.value != SCENE_NOT_SET).then_some(observed.value))
}

fn desired_row(inner: &Inner, adapter_id: u8, scene_id: u8, virtual_lamp_id: u8) -> SceneDesiredRowRecord {
    inner
        .scene_matrix
        .get(&(adapter_id, scene_id, virtual_lamp_id))
        .copied()
        .unwrap_or_default()
}

fn desired_raw(
    inner: &Inner,
    adapter_id: u8,
    scene_id: u8,
    virtual_lamp_id: u8,
) -> (bool, Option<DaliSceneTargetState>) {
    let row = desired_row(inner, adapter_id, scene_id, virtual_lamp_id);
    (row.included, row.target.filter(|_| row.included))
}

fn applied_raw(
    inner: &Inner,
    adapter_id: u8,
    scene_id: u8,
    virtual_lamp_id: u8,
) -> (bool, Option<DaliSceneTargetState>) {
    let Some(Some(level)) = scene_level_evidence(inner, adapter_id, scene_id, virtual_lamp_id)
    else {
        return (false, None);
    };
    let echo = inner
        .scene_applied_echo
        .get(&(adapter_id, scene_id, virtual_lamp_id));
    let color = match scene_colour_evidence(inner, adapter_id, scene_id, virtual_lamp_id) {
        SceneColourEvidence::Unread => echo.and_then(|e| e.color),
        SceneColourEvidence::NoColour => None,
        SceneColourEvidence::Colour(observed) => {
            let desired = desired_raw(inner, adapter_id, scene_id, virtual_lamp_id)
                .1
                .and_then(|t| t.color);
            Some(colour_from_observation(desired, &observed))
        }
    };
    let target = DaliSceneTargetState {
        power: echo.and_then(|e| e.power),
        level: Some(level),
        color,
    };
    (true, Some(target))
}

fn scene_colour_evidence(
    inner: &Inner,
    adapter_id: u8,
    scene_id: u8,
    virtual_lamp_id: u8,
) -> SceneColourEvidence {
    let Some(short) = binding_short(inner, adapter_id, virtual_lamp_id) else {
        return SceneColourEvidence::Unread;
    };
    let Some(device) = inner.physical_devices.get(&(adapter_id, short)) else {
        return SceneColourEvidence::Unread;
    };
    device
        .scene_colour_observed
        .get(usize::from(scene_id))
        .copied()
        .unwrap_or_default()
}

fn colour_from_observation(
    desired: Option<ColorValue>,
    observed: &crate::runtime::registry::physical_devices::SceneColourObservation,
) -> ColorValue {
    if let Some(desired) = desired {
        if observation_confirms(&desired, observed) {
            return desired;
        }
    }
    observed_colour_value(observed)
}

fn observed_colour_value(
    observed: &crate::runtime::registry::physical_devices::SceneColourObservation,
) -> ColorValue {
    let value = |slot: usize| observed.values.get(slot).copied().flatten();
    let level = |slot: usize| value(slot).map(|v| v as u8).unwrap_or(0);
    match observed.colour_type {
        SCENE_COLOUR_TYPE_TC => ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: value(0)
                .and_then(dali2rust_domain::registry::mirek_to_kelvin)
                .unwrap_or(0),
            ..ColorValue::default()
        },
        SCENE_COLOUR_TYPE_XY => ColorValue {
            mode: ColorMode::Xy,
            x: value(0).unwrap_or(0),
            y: value(1).unwrap_or(0),
            ..ColorValue::default()
        },
        SCENE_COLOUR_TYPE_RGBWAF => {
            let channel = |slot: usize| dim_level_to_srgb_channel(level(slot));
            let waf_present = (3..6).any(|slot| value(slot).is_some_and(|v| v != 0));
            ColorValue {
                mode: if waf_present {
                    ColorMode::Rgbwaf
                } else {
                    ColorMode::Rgb
                },
                r: channel(0),
                g: channel(1),
                b: channel(2),
                w: channel(3),
                a: channel(4),
                f: channel(5),
                ..ColorValue::default()
            }
        }
        _ => ColorValue {
            mode: ColorMode::Unknown,
            ..ColorValue::default()
        },
    }
}

fn observation_confirms(
    desired: &ColorValue,
    observed: &crate::runtime::registry::physical_devices::SceneColourObservation,
) -> bool {
    let value = |slot: usize| observed.values.get(slot).copied().flatten();
    let channel_confirms = |slot: usize, want: u8| {
        let encoded = u16::from(srgb_channel_to_dim_level(want));
        value(slot) == Some(encoded) || (want == 0 && value(slot).is_none())
    };
    match (desired.mode, observed.colour_type) {
        (ColorMode::Cct, SCENE_COLOUR_TYPE_TC) => {
            dali2rust_domain::registry::kelvin_to_mirek(desired.color_temperature_kelvin)
                .is_some_and(|mirek| value(0) == Some(mirek))
        }
        (ColorMode::Xy, SCENE_COLOUR_TYPE_XY) => {
            value(0) == Some(desired.x) && value(1) == Some(desired.y)
        }
        (ColorMode::Rgb | ColorMode::Rgbwaf, SCENE_COLOUR_TYPE_RGBWAF) => {
            let (w, a, f) = if desired.mode == ColorMode::Rgb {
                (0, 0, 0)
            } else {
                (desired.w, desired.a, desired.f)
            };
            [desired.r, desired.g, desired.b, w, a, f]
                .into_iter()
                .enumerate()
                .all(|(slot, want)| channel_confirms(slot, want))
        }
        _ => false,
    }
}

fn desired_state(inner: &Inner, adapter_id: u8, scene_id: u8, virtual_lamp_id: u8) -> SceneRowStateView {
    let (included, target) = desired_raw(inner, adapter_id, scene_id, virtual_lamp_id);
    state_from_target(included, target.as_ref())
}

fn applied_state(inner: &Inner, adapter_id: u8, scene_id: u8, virtual_lamp_id: u8) -> SceneRowStateView {
    let (included, target) = applied_raw(inner, adapter_id, scene_id, virtual_lamp_id);
    state_from_target(included, target.as_ref())
}

fn scene_row_dirty(inner: &Inner, adapter_id: u8, scene_id: u8, virtual_lamp_id: u8) -> bool {
    desired_raw(inner, adapter_id, scene_id, virtual_lamp_id)
        != applied_raw(inner, adapter_id, scene_id, virtual_lamp_id)
}

fn scene_dirty(inner: &Inner, adapter_id: u8, scene_id: u8) -> bool {
    (0..VIRTUAL_LAMP_COUNT).any(|vl| scene_row_dirty(inner, adapter_id, scene_id, vl))
}

pub(crate) fn scene_record(inner: &Inner, adapter_id: u8, scene_id: u8) -> SceneRecord {
    inner
        .scenes
        .get(&(adapter_id, scene_id))
        .cloned()
        .unwrap_or_default()
}

fn build_scene_view(inner: &Inner, adapter_id: u8, scene_id: u8) -> SceneView {
    let record = scene_record(inner, adapter_id, scene_id);
    let row_count_included = (0..VIRTUAL_LAMP_COUNT)
        .filter(|vl| desired_row(inner, adapter_id, scene_id, *vl).included)
        .count() as u8;
    SceneView {
        adapter_id,
        scene_id,
        name: record.name.as_str().to_string(),
        ha_select_enabled: record.ha_select_enabled,
        row_count_included,
        dirty: scene_dirty(inner, adapter_id, scene_id),
    }
}

fn build_matrix_row(inner: &Inner, adapter_id: u8, scene_id: u8, virtual_lamp_id: u8) -> SceneMatrixRowView {
    let desired = desired_state(inner, adapter_id, scene_id, virtual_lamp_id);
    let applied = applied_state(inner, adapter_id, scene_id, virtual_lamp_id);
    let dirty = desired != applied;
    let name = inner
        .lamps
        .get(&(adapter_id, virtual_lamp_id))
        .map(|lamp| lamp.name.as_str().to_string())
        .unwrap_or_default();
    SceneMatrixRowView {
        virtual_lamp_id,
        name,
        capabilities: inner.vl_capabilities(adapter_id, virtual_lamp_id),
        desired,
        applied,
        dirty,
    }
}

fn seed_scene_rows_for_short(inner: &mut Inner, adapter_id: u8, short: u8, scene_id: u8) -> bool {
    let mut changed = false;
    for virtual_lamp_id in 0..VIRTUAL_LAMP_COUNT {
        if binding_short(inner, adapter_id, virtual_lamp_id) != Some(short) {
            continue;
        }
        let Some(applied_level) = scene_level_evidence(inner, adapter_id, scene_id, virtual_lamp_id)
        else {
            continue;
        };
        let entry = inner
            .scene_matrix
            .entry((adapter_id, scene_id, virtual_lamp_id))
            .or_default();
        if entry.desired_seeded {
            continue;
        }
        entry.included = applied_level.is_some();
        entry.target = applied_level.map(|level| DaliSceneTargetState {
            power: None,
            level: Some(level),
            color: None,
        });
        entry.desired_seeded = true;
        changed = true;
    }
    changed
}

fn store_scene_echo_for_short(
    inner: &mut Inner,
    adapter_id: u8,
    short: u8,
    scene_id: u8,
    echo: Option<&DaliSceneTargetState>,
) -> bool {
    let mut changed = false;
    for virtual_lamp_id in 0..VIRTUAL_LAMP_COUNT {
        if binding_short(inner, adapter_id, virtual_lamp_id) != Some(short) {
            continue;
        }
        let key = (adapter_id, scene_id, virtual_lamp_id);
        match echo {
            Some(state) => {
                if inner.scene_applied_echo.insert(key, *state) != Some(*state) {
                    changed = true;
                }
            }
            None => changed |= inner.scene_applied_echo.remove(&key).is_some(),
        }
    }
    changed
}

impl RegistryStore {
    pub(crate) fn apply_scene_metadata_patch(
        &self,
        adapter_id: u8,
        scene_id: u8,
        patch_mask: u8,
        name: Option<&str>,
        ha_select_enabled: bool,
    ) -> bool {
        use dali2rust_contracts::msg::SceneMetadataUpdateCommand;

        let mut inner = self.write_inner();
        if !inner.adapter_exists(adapter_id) || scene_id >= SCENE_COUNT {
            return false;
        }
        let entry = inner.scenes.entry((adapter_id, scene_id)).or_default();
        let mut changed = false;
        if patch_mask & SceneMetadataUpdateCommand::PATCH_NAME != 0 {
            let next = fixed_text_64(name.unwrap_or(""));
            if entry.name != next {
                entry.name = next;
                changed = true;
            }
        }
        if patch_mask & SceneMetadataUpdateCommand::PATCH_HA_SELECT_ENABLED != 0
            && entry.ha_select_enabled != ha_select_enabled
        {
            entry.ha_select_enabled = ha_select_enabled;
            changed = true;
        }
        if changed {
            inner.scenes_metadata_revision = inner.scenes_metadata_revision.wrapping_add(1);
        }
        drop(inner);
        if changed {
            self.dirty.mark_scene_dirty(adapter_id, scene_id);
        }
        changed
    }

    pub(crate) fn apply_scene_matrix_rows(
        &self,
        adapter_id: u8,
        scene_id: u8,
        rows: &[SceneMatrixDesiredRow],
    ) -> bool {
        let mut inner = self.write_inner();
        if !inner.adapter_exists(adapter_id) || scene_id >= SCENE_COUNT {
            return false;
        }
        let mut changed = false;
        for row in rows {
            let entry = inner
                .scene_matrix
                .entry((adapter_id, scene_id, row.virtual_lamp_id))
                .or_default();
            if entry.included != row.included || entry.target != row.target {
                entry.included = row.included;
                entry.target = row.target;
                changed = true;
            }
            if !entry.desired_seeded {
                entry.desired_seeded = true;
                changed = true;
            }
        }
        if changed {
            inner.scenes_matrix_desired_revision =
                inner.scenes_matrix_desired_revision.wrapping_add(1);
        }
        drop(inner);
        if changed {
            self.dirty.mark_scene_dirty(adapter_id, scene_id);
        }
        changed
    }

    pub(crate) fn apply_scene_programmed_readback(
        &self,
        adapter_id: u8,
        short_address: u8,
        scene_id: u8,
        scene_level: Option<u8>,
        echo: Option<&DaliSceneTargetState>,
    ) -> bool {
        let mut inner = self.write_inner();
        if !inner.adapter_exists(adapter_id) || scene_id >= SCENE_COUNT {
            return false;
        }
        let Some(record) = inner.physical_devices.get_mut(&(adapter_id, short_address)) else {
            return false;
        };
        let mut level_committed = false;
        let mut level_changed = false;
        if let Some(level) = scene_level {
            level_changed =
                write_scene_level_read(&mut record.attributes, scene_id, level, registry_unix_ms());
            level_committed = true;
        }
        let echo_changed =
            store_scene_echo_for_short(&mut inner, adapter_id, short_address, scene_id, echo);
        let seeded = seed_scene_rows_for_short(&mut inner, adapter_id, short_address, scene_id);
        let changed = level_committed || echo_changed || seeded;
        if level_committed {
            inner.physical_devices_revision = inner.physical_devices_revision.saturating_add(1);
        }
        if changed {
            inner.scenes_matrix_desired_revision =
                inner.scenes_matrix_desired_revision.wrapping_add(1);
        }
        drop(inner);
        if level_changed {
            self.dirty.mark_physical_devices_dirty(adapter_id);
        }
        if seeded {
            self.dirty.mark_scene_dirty(adapter_id, scene_id);
        }
        changed
    }

    pub(crate) fn apply_foreign_scene_write(&self, adapter_id: u8, write: ForeignSceneWrite) -> Vec<u8> {
        let mut inner = self.write_inner();
        if !inner.adapter_exists(adapter_id) || write.scene_id >= SCENE_COUNT {
            return Vec::new();
        }
        let now = registry_unix_ms();
        let mut changed = Vec::new();
        for (short, certain) in foreign_write_targets(&inner, adapter_id, &write) {
            let Some(record) = inner.physical_devices.get_mut(&(adapter_id, short)) else {
                continue;
            };
            let level_changed = if write.removal && certain {
                write_scene_level_written(&mut record.attributes, write.scene_id, SCENE_NOT_SET, now)
            } else {
                forget_scene_level(&mut record.attributes, write.scene_id)
            };
            let echo_changed = store_scene_echo_for_short(&mut inner, adapter_id, short, write.scene_id, None);
            if level_changed || echo_changed {
                changed.push(short);
            }
        }
        if !changed.is_empty() {
            inner.physical_devices_revision = inner.physical_devices_revision.saturating_add(1);
            inner.scenes_matrix_desired_revision = inner.scenes_matrix_desired_revision.wrapping_add(1);
        }
        drop(inner);
        if !changed.is_empty() {
            self.dirty.mark_physical_devices_dirty(adapter_id);
        }
        changed
    }

    pub(crate) fn seed_scene_matrix_from_levels_read(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> u16 {
        let mut inner = self.write_inner();
        if !inner.adapter_exists(adapter_id) {
            return 0;
        }
        let mut changed_mask = 0u16;
        for scene_id in 0..SCENE_COUNT {
            if seed_scene_rows_for_short(&mut inner, adapter_id, short_address, scene_id) {
                changed_mask |= 1u16 << scene_id;
            }
        }
        if changed_mask != 0 {
            inner.scenes_matrix_desired_revision =
                inner.scenes_matrix_desired_revision.wrapping_add(1);
        }
        drop(inner);
        for scene_id in 0..SCENE_COUNT {
            if changed_mask & (1u16 << scene_id) != 0 {
                self.dirty.mark_scene_dirty(adapter_id, scene_id);
            }
        }
        changed_mask
    }

    fn scene_view_internal(&self, adapter_id: u8, scene_id: u8) -> Option<SceneView> {
        let inner = self.read_inner();
        if !inner.adapter_exists(adapter_id) || scene_id >= SCENE_COUNT {
            return None;
        }
        Some(build_scene_view(&inner, adapter_id, scene_id))
    }

    fn scene_matrix_view_internal(&self, adapter_id: u8, scene_id: u8) -> Option<SceneMatrixView> {
        let inner = self.read_inner();
        if !inner.adapter_exists(adapter_id) || scene_id >= SCENE_COUNT {
            return None;
        }
        let rows = (0..VIRTUAL_LAMP_COUNT)
            .map(|vl| build_matrix_row(&inner, adapter_id, scene_id, vl))
            .collect();
        Some(SceneMatrixView {
            adapter_id,
            scene_id,
            rows,
        })
    }

    fn scene_apply_snapshot_internal(&self, adapter_id: u8, scene_id: u8) -> Option<SceneApplySnapshot> {
        let inner = self.read_inner();
        if !inner.adapter_exists(adapter_id) || scene_id >= SCENE_COUNT {
            return None;
        }
        let rows = (0..VIRTUAL_LAMP_COUNT)
            .map(|vl| {
                let (desired_included, desired_target) =
                    desired_raw(&inner, adapter_id, scene_id, vl);
                let (applied_included, applied_target) =
                    applied_raw(&inner, adapter_id, scene_id, vl);
                SceneApplyRowView {
                    virtual_lamp_id: vl,
                    desired_included,
                    desired_target,
                    applied_included,
                    applied_target,
                    binding_short: binding_short(&inner, adapter_id, vl),
                }
            })
            .collect();
        Some(SceneApplySnapshot {
            adapter_id,
            scene_id,
            rows,
        })
    }
}

impl SceneReadPort for RegistryStore {
    fn scene_view(&self, adapter_id: u8, scene_id: u8) -> Option<SceneView> {
        self.scene_view_internal(adapter_id, scene_id)
    }

    fn list_scene_views(&self, adapter_id: u8) -> Vec<SceneView> {
        (0..SCENE_COUNT)
            .filter_map(|scene_id| self.scene_view_internal(adapter_id, scene_id))
            .collect()
    }

    fn scene_matrix_view(&self, adapter_id: u8, scene_id: u8) -> Option<SceneMatrixView> {
        self.scene_matrix_view_internal(adapter_id, scene_id)
    }

    fn scene_apply_snapshot(&self, adapter_id: u8, scene_id: u8) -> Option<SceneApplySnapshot> {
        self.scene_apply_snapshot_internal(adapter_id, scene_id)
    }
}

impl RegistryStore {
    pub(crate) fn note_scene_recalled(
        &self,
        adapter_id: u8,
        scene_id: u8,
        scope: dali2rust_contracts::msg::DaliTargetScope,
    ) {
        let mut inner = self.write_inner();
        if scope == dali2rust_contracts::msg::DaliTargetScope::Broadcast {
            inner.active_scene.insert(adapter_id, scene_id);
        } else {
            inner.active_scene.remove(&adapter_id);
        }
    }

    pub(crate) fn clear_active_scene_unless_from_scene(
        &self,
        adapter_id: u8,
        last_dapc: Option<dali2rust_contracts::msg::LastDapcSource>,
    ) {
        let Some(source) = last_dapc else {
            return;
        };
        if source == dali2rust_contracts::msg::LastDapcSource::Scene {
            return;
        }
        self.write_inner().active_scene.remove(&adapter_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::registry::physical_devices::PhysicalDeviceRecord;
    use dali2rust_bsp::psram::PsramBox;
    use dali2rust_contracts::msg::SceneMetadataUpdateCommand;
    use dali2rust_domain::registry::{AttributeSource, ObservedValue};

    const ADAPTER: u8 = 0;
    const SCENE: u8 = 3;
    const VL: u8 = 1;
    const SHORT: u8 = 5;

    fn store_with_bound_lamp() -> RegistryStore {
        let store = RegistryStore::with_adapter_count(1);
        {
            let mut g = store.inner.write().expect("registry lock");
            g.physical_devices
                .insert((ADAPTER, SHORT), PsramBox::new(PhysicalDeviceRecord::empty(SHORT)));
            g.lamps.entry((ADAPTER, VL)).or_default().binding_short = Some(SHORT);
        }
        store
    }

    #[test]
    fn reprogramming_an_identical_scene_level_does_not_redirty_pd() {
        let store = store_with_bound_lamp();
        assert!(store.apply_scene_programmed_readback(ADAPTER, SHORT, SCENE, Some(120), None));
        assert!(store.dirty.take_physical_devices_dirty_for(0));
        let _ = store.dirty.take_scenes_dirty(ADAPTER);

        assert!(store.apply_scene_programmed_readback(ADAPTER, SHORT, SCENE, Some(120), None));
        assert!(
            !store.dirty.take_physical_devices_dirty_for(0),
            "re-programming the level the record already holds must not rewrite flash"
        );
    }

    fn desired_level_row(virtual_lamp_id: u8, level: u8) -> SceneMatrixDesiredRow {
        SceneMatrixDesiredRow {
            virtual_lamp_id,
            included: true,
            target: Some(DaliSceneTargetState {
                power: Some(PowerState::On),
                level: Some(level),
                color: None,
            }),
        }
    }

    #[test]
    fn metadata_patch_updates_view_and_rejects_bad_scene_id() {
        let store = store_with_bound_lamp();
        assert!(store.apply_scene_metadata_patch(
            ADAPTER,
            SCENE,
            SceneMetadataUpdateCommand::PATCH_NAME,
            Some("Reading"),
            true,
        ));
        let view = store.scene_view_internal(ADAPTER, SCENE).expect("scene view");
        assert_eq!(view.name, "Reading");
        assert!(view.ha_select_enabled);
        assert!(!store.apply_scene_metadata_patch(ADAPTER, 16, 0xFF, Some("x"), false));
    }

    #[test]
    fn matrix_rows_apply_marks_seeded_and_dirty_until_readback() {
        let store = store_with_bound_lamp();
        assert!(store.apply_scene_matrix_rows(ADAPTER, SCENE, &[desired_level_row(VL, 100)]));
        let view = store.scene_view_internal(ADAPTER, SCENE).expect("scene view");
        assert_eq!(view.row_count_included, 1);
        assert!(view.dirty, "no applied evidence yet — row must be dirty");
        let g = store.inner.read().expect("registry lock");
        assert!(g.scene_matrix[&(ADAPTER, SCENE, VL)].desired_seeded);
    }

    #[test]
    fn programmed_readback_projects_applied_level_and_echo() {
        let store = store_with_bound_lamp();
        store.apply_scene_matrix_rows(ADAPTER, SCENE, &[desired_level_row(VL, 100)]);
        let echo = DaliSceneTargetState {
            power: Some(PowerState::On),
            level: Some(100),
            color: None,
        };
        assert!(store.apply_scene_programmed_readback(ADAPTER, SHORT, SCENE, Some(100), Some(&echo)));
        let matrix = store
            .scene_matrix_view_internal(ADAPTER, SCENE)
            .expect("matrix");
        let row = &matrix.rows[VL as usize];
        assert!(row.applied.included);
        assert_eq!(row.applied.level, Some(100));
        assert_eq!(row.applied.power.as_deref(), Some("on"));
        assert!(!row.dirty, "desired == applied after confirmed readback");
    }

    #[test]
    fn unconfirmed_readback_keeps_applied_and_row_dirty() {
        let store = store_with_bound_lamp();
        store.apply_scene_matrix_rows(ADAPTER, SCENE, &[desired_level_row(VL, 100)]);
        let echo = DaliSceneTargetState {
            power: Some(PowerState::On),
            level: Some(100),
            color: None,
        };
        assert!(store.apply_scene_programmed_readback(ADAPTER, SHORT, SCENE, None, Some(&echo)));
        let matrix = store
            .scene_matrix_view_internal(ADAPTER, SCENE)
            .expect("matrix");
        let row = &matrix.rows[VL as usize];
        assert!(!row.applied.included, "no readback — applied stays excluded");
        assert!(row.dirty);
    }

    #[test]
    fn clear_readback_removes_echo_and_excludes_applied() {
        let store = store_with_bound_lamp();
        store.apply_scene_matrix_rows(ADAPTER, SCENE, &[desired_level_row(VL, 100)]);
        let echo = DaliSceneTargetState {
            power: Some(PowerState::On),
            level: Some(100),
            color: None,
        };
        store.apply_scene_programmed_readback(ADAPTER, SHORT, SCENE, Some(100), Some(&echo));
        assert!(store.apply_scene_programmed_readback(
            ADAPTER,
            SHORT,
            SCENE,
            Some(SCENE_NOT_SET),
            None
        ));
        let matrix = store
            .scene_matrix_view_internal(ADAPTER, SCENE)
            .expect("matrix");
        let row = &matrix.rows[VL as usize];
        assert!(!row.applied.included);
        assert_eq!(row.applied.level, None);
        assert_eq!(row.applied.power, None, "echo removed on clear");
    }

    #[test]
    fn first_readback_seeds_unseeded_rows_and_preserves_user_rows() {
        let store = store_with_bound_lamp();
        store.apply_scene_matrix_rows(ADAPTER, SCENE, &[desired_level_row(VL, 180)]);
        {
            let mut g = store.inner.write().expect("registry lock");
            let record = g
                .physical_devices
                .get_mut(&(ADAPTER, SHORT))
                .expect("physical device");
            write_scene_level_read(&mut record.attributes, SCENE, 100, 1);
        }
        let mask = store.seed_scene_matrix_from_levels_read(ADAPTER, SHORT);
        assert_eq!(mask, 0, "seeded user row must not be rewritten (REG-031)");
        let matrix = store
            .scene_matrix_view_internal(ADAPTER, SCENE)
            .expect("matrix");
        let row = &matrix.rows[VL as usize];
        assert_eq!(row.desired.level, Some(180), "user desired preserved");
        assert_eq!(row.applied.level, Some(100));
        assert!(row.dirty, "desired differs from applied until apply succeeds");

        {
            let mut g = store.inner.write().expect("registry lock");
            g.lamps.entry((ADAPTER, 2)).or_default().binding_short = Some(SHORT);
        }
        let mask = store.seed_scene_matrix_from_levels_read(ADAPTER, SHORT);
        assert_eq!(mask, 1u16 << SCENE);
        let matrix = store
            .scene_matrix_view_internal(ADAPTER, SCENE)
            .expect("matrix");
        let seeded = &matrix.rows[2];
        assert!(seeded.desired.included);
        assert_eq!(seeded.desired.level, Some(100), "desired := applied");
        assert_eq!(seeded.desired.color_mode, None, "color stays null on seed");
        assert!(!seeded.dirty);
    }

    fn store_with_group_members(members: &[(u8, Option<u16>)]) -> RegistryStore {
        let store = RegistryStore::with_adapter_count(1);
        {
            let mut g = store.inner.write().expect("registry lock");
            for &(short, membership) in members {
                let mut record = PhysicalDeviceRecord::empty(short);
                record.attributes.groups.membership = membership.map(|value| ObservedValue {
                    value,
                    source: AttributeSource::Readback,
                    last_read_ms: Some(1),
                    last_write_confirmed_ms: None,
                });
                g.physical_devices.insert((ADAPTER, short), PsramBox::new(record));
            }
        }
        for &(short, _) in members {
            assert!(store.apply_scene_programmed_readback(ADAPTER, short, SCENE, Some(120), None));
        }
        store
    }

    fn scene_level(store: &RegistryStore, short: u8) -> Option<u8> {
        scene_evidence(store, short).map(|(value, _)| value)
    }

    fn scene_evidence(store: &RegistryStore, short: u8) -> Option<(u8, AttributeSource)> {
        let g = store.inner.read().expect("registry lock");
        g.physical_devices[&(ADAPTER, short)].attributes.scenes.levels[SCENE as usize]
            .as_ref()
            .map(|observed| (observed.value, observed.source))
    }

    #[test]
    fn a_foreign_group_removal_is_certain_only_where_membership_is_known() {
        const GROUP: u8 = 4;
        let store = store_with_group_members(&[(1, Some(1 << GROUP)), (2, Some(0)), (3, None)]);
        let write = ForeignSceneWrite {
            scope: DaliTargetScope::Group,
            short_address: None,
            group_id: Some(GROUP),
            scene_id: SCENE,
            removal: true,
        };
        assert_eq!(store.apply_foreign_scene_write(ADAPTER, write), vec![1, 3]);
        assert_eq!(
            scene_evidence(&store, 1),
            Some((SCENE_NOT_SET, AttributeSource::WriteConfirmed)),
            "a known member left the scene, known from the write, not from a read"
        );
        assert_eq!(scene_level(&store, 2), Some(120), "a known non-member is untouched");
        assert_eq!(scene_level(&store, 3), None, "unknown membership: the level is only unread");
    }

    #[test]
    fn a_foreign_broadcast_write_leaves_every_level_unread() {
        let store = store_with_group_members(&[(1, None), (2, Some(0))]);
        let write = ForeignSceneWrite {
            scope: DaliTargetScope::Broadcast,
            short_address: None,
            group_id: None,
            scene_id: SCENE,
            removal: false,
        };
        assert_eq!(store.apply_foreign_scene_write(ADAPTER, write), vec![1, 2]);
        assert_eq!(scene_level(&store, 1), None);
        assert_eq!(scene_level(&store, 2), None);
        assert!(store.dirty.take_physical_devices_dirty_for(ADAPTER));
    }
}
