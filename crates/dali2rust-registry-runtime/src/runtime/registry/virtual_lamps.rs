use dali2rust_contracts::msg::{fixed_text_64, FixedText64};
use dali2rust_contracts::msg::{VirtualLampConfigUpdateCommand};
use dali2rust_domain::registry::{
    AdapterSnapshot, VirtualLampCapabilityReadPort, VirtualLampCapabilityView, VirtualLampReadPort,
    VirtualLampSnapshot, VirtualLampView,
};

use crate::runtime::registry::store::RegistryStore;
use crate::runtime::registry::views::virtual_lamp_view_from_parts;

#[derive(Clone)]
pub(crate) struct VlRecord {
    pub name: FixedText64,
    pub ha_entity_enabled: bool,
    pub binding_short: Option<u8>,
}

impl Default for VlRecord {
    fn default() -> Self {
        Self {
            name: FixedText64::new(),
            ha_entity_enabled: true,
            binding_short: None,
        }
    }
}

impl RegistryStore {
    pub(crate) fn apply_virtual_lamp_bind(
        &self,
        adapter_id: u8,
        virtual_lamp_id: u8,
        physical_short_address: u8,
    ) -> bool {
        let mut g = self.write_inner();
        if !g
            .physical_devices
            .contains_key(&(adapter_id, physical_short_address))
        {
            return false;
        }
        if g.lamps.iter().any(|((aid, vlid), record)| {
            *aid == adapter_id
                && *vlid != virtual_lamp_id
                && record.binding_short == Some(physical_short_address)
        }) {
            return false;
        }
        let e = g
            .lamps
            .entry((adapter_id, virtual_lamp_id))
            .or_default();
        let previous_short = e.binding_short.replace(physical_short_address);
        let mut groups_changed = false;
        if previous_short.is_some_and(|prev| prev != physical_short_address) {
            groups_changed = super::groups::forget_adopted_desired_on_binding_change(
                &mut g,
                adapter_id,
                virtual_lamp_id,
            );
        }
        groups_changed |= super::groups::seed_desired_for_new_binding(
            &mut g,
            adapter_id,
            virtual_lamp_id,
            physical_short_address,
        );
        drop(g);
        self.dirty.mark_virtual_lamps_dirty(adapter_id);
        if groups_changed {
            self.dirty.mark_groups_dirty(adapter_id);
        }
        true
    }

    pub(crate) fn apply_virtual_lamp_unbind(&self, adapter_id: u8, virtual_lamp_id: u8) {
        let mut g = self.write_inner();
        if let Some(e) = g.lamps.get_mut(&(adapter_id, virtual_lamp_id)) {
            e.binding_short = None;
        }
        let groups_changed =
            super::groups::forget_adopted_desired_on_binding_change(&mut g, adapter_id, virtual_lamp_id);
        drop(g);
        self.dirty.mark_virtual_lamps_dirty(adapter_id);
        if groups_changed {
            self.dirty.mark_groups_dirty(adapter_id);
        }
    }

    pub(crate) fn apply_virtual_lamp_delete(&self, adapter_id: u8, virtual_lamp_id: u8) -> bool {
        let mut g = self.write_inner();
        let existed = g.lamps.remove(&(adapter_id, virtual_lamp_id)).is_some();
        let groups_changed = g
            .group_matrix
            .remove(&(adapter_id, virtual_lamp_id))
            .is_some();
        let mut scenes_changed = 0u16;
        for scene_id in 0..super::scenes::SCENE_COUNT {
            let key = (adapter_id, scene_id, virtual_lamp_id);
            if g.scene_matrix.remove(&key).is_some() || g.scene_applied_echo.remove(&key).is_some() {
                scenes_changed |= 1u16 << scene_id;
            }
        }
        drop(g);
        if !existed && !groups_changed && scenes_changed == 0 {
            return false;
        }
        self.dirty.mark_virtual_lamps_dirty(adapter_id);
        if groups_changed {
            self.dirty.mark_groups_dirty(adapter_id);
        }
        for scene_id in 0..super::scenes::SCENE_COUNT {
            if scenes_changed & (1u16 << scene_id) != 0 {
                self.dirty.mark_scene_dirty(adapter_id, scene_id);
            }
        }
        true
    }

    pub(crate) fn apply_virtual_lamp_config_patch(
        &self,
        adapter_id: u8,
        virtual_lamp_id: u8,
        patch_mask: u8,
        name: Option<&str>,
        ha_entity_enabled: bool,
    ) {
        let mut g = self.write_inner();
        let e = g
            .lamps
            .entry((adapter_id, virtual_lamp_id))
            .or_default();
        if patch_mask & VirtualLampConfigUpdateCommand::PATCH_NAME != 0 {
            if let Some(n) = name {
                e.name = fixed_text_64(n);
            }
        }
        if patch_mask & VirtualLampConfigUpdateCommand::PATCH_HA_ENTITY_ENABLED != 0 {
            e.ha_entity_enabled = ha_entity_enabled;
        }
        drop(g);
        self.dirty.mark_virtual_lamps_dirty(adapter_id);
    }

    fn virtual_lamp_view_internal(&self, adapter_id: u8, lamp_id: u8) -> VirtualLampView {
        let g = self.read_inner();
        let vl = g
            .lamps
            .get(&(adapter_id, lamp_id))
            .cloned()
            .unwrap_or_default();
        let physical_rec = vl
            .binding_short
            .and_then(|sa| g.physical_devices.get(&(adapter_id, sa)))
            .map(|boxed| boxed.as_ref());
        virtual_lamp_view_from_parts(adapter_id, lamp_id, &vl, physical_rec)
    }

    pub(crate) fn collect_virtual_lamp_ids_for_adapter(&self, adapter_id: u8) -> Vec<u8> {
        let g = self.read_inner();
        let mut ids: Vec<u8> = g
            .lamps
            .keys()
            .filter_map(|(aid, vlid)| {
                if *aid != adapter_id {
                    return None;
                }
                Some(*vlid)
            })
            .collect();
        ids.sort_unstable();
        ids
    }

    pub(crate) fn internal_virtual_lamp_snapshot(
        &self,
        adapter_id: u8,
        virtual_lamp_id: u8,
    ) -> VirtualLampSnapshot {
        let g = self.read_inner();
        let vl = g.lamps.get(&(adapter_id, virtual_lamp_id));
        let name = vl.map(|r| r.name.as_str().to_string()).unwrap_or_default();
        let runtime_level = vl
            .and_then(|r| r.binding_short)
            .and_then(|sa| g.physical_devices.get(&(adapter_id, sa)))
            .map(|pr| pr.runtime_level.unwrap_or(0))
            .unwrap_or(0);
        drop(g);
        VirtualLampSnapshot {
            name,
            runtime_level,
        }
    }

    pub(crate) fn internal_virtual_lamp_binding_short(
        &self,
        adapter_id: u8,
        virtual_lamp_id: u8,
    ) -> Option<u8> {
        let g = self.read_inner();
        g.lamps
            .get(&(adapter_id, virtual_lamp_id))
            .and_then(|r| r.binding_short)
    }

    pub(crate) fn internal_virtual_lamp_bound_to_short(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<u8> {
        let g = self.read_inner();
        g.lamps
            .iter()
            .filter(|((aid, _), record)| {
                *aid == adapter_id && record.binding_short == Some(short_address)
            })
            .map(|((_, lamp_id), _)| *lamp_id)
            .min()
    }

    pub(crate) fn internal_adapter_snapshot(&self, adapter_id: u8) -> AdapterSnapshot {
        let g = self.read_inner();
        g.adapters
            .get(adapter_id as usize)
            .map(|r| AdapterSnapshot { enabled: r.enabled })
            .unwrap_or_default()
    }
}

impl VirtualLampReadPort for RegistryStore {
    fn virtual_lamp_view(&self, adapter_id: u8, lamp_id: u8) -> VirtualLampView {
        self.virtual_lamp_view_internal(adapter_id, lamp_id)
    }

    fn list_virtual_lamp_ids(&self, adapter_id: u8) -> Vec<u8> {
        self.collect_virtual_lamp_ids_for_adapter(adapter_id)
    }

    fn list_virtual_lamp_views(&self, adapter_id: u8) -> Vec<VirtualLampView> {
        let g = self.read_inner();
        let mut ids: Vec<u8> = g
            .lamps
            .keys()
            .filter_map(|(aid, vlid)| {
                if *aid != adapter_id {
                    return None;
                }
                Some(*vlid)
            })
            .collect();
        ids.sort_unstable();
        ids.iter()
            .map(|&id| {
                let vl = g.lamps.get(&(adapter_id, id)).cloned().unwrap_or_default();
                let physical_rec = vl
                    .binding_short
                    .and_then(|sa| g.physical_devices.get(&(adapter_id, sa)))
            .map(|boxed| boxed.as_ref());
                virtual_lamp_view_from_parts(adapter_id, id, &vl, physical_rec)
            })
            .collect()
    }

    fn physical_short_on_other_adapter(&self, adapter_id: u8, short_address: u8) -> bool {
        RegistryStore::physical_short_address_on_other_adapter(self, adapter_id, short_address)
    }
}

impl VirtualLampCapabilityReadPort for RegistryStore {
    fn virtual_lamp_capability_view(
        &self,
        adapter_id: u8,
        lamp_id: u8,
    ) -> VirtualLampCapabilityView {
        self.read_inner().vl_capability_view(adapter_id, lamp_id)
    }

    fn list_virtual_lamp_capability_views(
        &self,
        adapter_id: u8,
    ) -> Vec<VirtualLampCapabilityView> {
        let g = self.read_inner();
        let mut ids: Vec<u8> = g
            .lamps
            .keys()
            .filter_map(|(aid, vlid)| (*aid == adapter_id).then_some(*vlid))
            .collect();
        ids.sort_unstable();
        ids.into_iter()
            .map(|id| g.vl_capability_view(adapter_id, id))
            .collect()
    }
}
