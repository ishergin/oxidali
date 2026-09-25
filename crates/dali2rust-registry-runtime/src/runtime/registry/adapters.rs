use crate::runtime::registry::store::RegistryStore;
use dali2rust_contracts::msg::{fixed_text_64, FixedText64};
use dali2rust_domain::registry::{AdapterEnabledReadPort, AdapterReadPort, AdapterView};

#[derive(Clone, Debug, Default)]
pub(crate) struct AdapterRow {
    pub name: FixedText64,
    pub enabled: bool,
    pub commands: u64,
    pub timeouts: u64,
    pub errors: u64,
}

impl AdapterRow {
    pub(crate) fn to_view(&self, adapter_id: u8) -> AdapterView {
        AdapterView {
            adapter_id,
            name: self.name.as_str().to_string(),
            enabled: self.enabled,
            commands: self.commands,
            timeouts: self.timeouts,
            errors: self.errors,
        }
    }
}

impl AdapterEnabledReadPort for RegistryStore {
    fn adapter_enabled(&self, adapter_id: u8) -> bool {
        self.internal_adapter_snapshot(adapter_id).enabled
    }
}

impl RegistryStore {
    pub(crate) fn apply_adapter_settings_from_command(
        &self,
        adapter_id: u8,
        patch_mask: u8,
        name: Option<&str>,
        enabled: bool,
    ) -> bool {
        let mut g = self.write_inner();
        let Some(row) = g.adapters.get_mut(adapter_id as usize) else {
            return false;
        };
        if patch_mask & 1 != 0 {
            if let Some(n) = name {
                row.name = fixed_text_64(n);
            }
        }
        if patch_mask & 2 != 0 {
            row.enabled = enabled;
        }
        drop(g);
        self.dirty
            .adapters
            .store(true, std::sync::atomic::Ordering::Release);
        true
    }

    pub(crate) fn adapter_row_name_enabled(&self, adapter_id: u8) -> Option<(FixedText64, bool)> {
        let g = self.read_inner();
        let r = g.adapters.get(adapter_id as usize)?;
        Some((r.name.clone(), r.enabled))
    }

    pub(crate) fn adapter_count_inner(&self) -> u8 {
        self.read_inner().adapters.len() as u8
    }
}

impl AdapterReadPort for RegistryStore {
    fn adapter_count(&self) -> u8 {
        self.adapter_count_inner()
    }

    fn adapter_view(&self, id: u8) -> Option<AdapterView> {
        let g = self.read_inner();
        g.adapters.get(id as usize).map(|r| r.to_view(id))
    }

    fn list_adapter_views(&self) -> Vec<AdapterView> {
        let g = self.read_inner();
        g.adapters
            .iter()
            .enumerate()
            .map(|(i, r)| r.to_view(i as u8))
            .collect()
    }
}
