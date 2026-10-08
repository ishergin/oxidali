use std::sync::atomic::{AtomicU32, Ordering};

use dali2rust_platform::slice_store::SliceKey;

use super::store::MAX_DIRTY_ADAPTERS;

pub(crate) const ADAPTERS: u32 = 1 << 0;
pub(crate) const HCL_SCHEDULES: u32 = 1 << 1;
pub(crate) const POLLER_SETTINGS: u32 = 1 << 2;
pub(crate) const DALI_SETTINGS: u32 = 1 << 3;
pub(crate) const REDUNDANCY_SETTINGS: u32 = 1 << 4;
pub(crate) const POLICIES: u32 = 1 << 5;
pub(crate) const HOME_ASSISTANT_SETTINGS: u32 = 1 << 6;
const FIRST_INPUT_DEVICE_BANK: u32 = 8;
const ALL_INPUT_DEVICE_BANKS: u8 = (1 << SliceKey::INPUT_DEVICE_BANKS) - 1;
pub(crate) const INPUT_DEVICES: u32 = (ALL_INPUT_DEVICE_BANKS as u32) << FIRST_INPUT_DEVICE_BANK;

const GLOBALS: [(u32, SliceKey); 7] = [
    (ADAPTERS, SliceKey::Adapters),
    (HCL_SCHEDULES, SliceKey::HclSchedules),
    (POLLER_SETTINGS, SliceKey::PollerSettings),
    (DALI_SETTINGS, SliceKey::DaliSettings),
    (REDUNDANCY_SETTINGS, SliceKey::RedundancySettings),
    (POLICIES, SliceKey::Policies),
    (HOME_ASSISTANT_SETTINGS, SliceKey::HomeAssistantSettings),
];

fn bit(index: u8) -> u32 {
    1u32.checked_shl(u32::from(index)).unwrap_or(0)
}

#[derive(Default)]
pub(crate) struct WithheldSlices {
    global: AtomicU32,
    groups: AtomicU32,
    virtual_lamps: AtomicU32,
    scenes: [AtomicU32; MAX_DIRTY_ADAPTERS],
}

impl WithheldSlices {
    pub(crate) fn settle(&self, key: SliceKey, unread: bool) {
        let Some((cell, mask)) = self.cell(key) else {
            return;
        };
        if unread {
            cell.fetch_or(mask, Ordering::AcqRel);
        } else {
            cell.fetch_and(!mask, Ordering::AcqRel);
        }
    }

    fn cell(&self, key: SliceKey) -> Option<(&AtomicU32, u32)> {
        let global = |mask| Some((&self.global, mask));
        match key {
            SliceKey::Groups { adapter_id } => Some((&self.groups, bit(adapter_id))),
            SliceKey::VirtualLamps { adapter_id } => Some((&self.virtual_lamps, bit(adapter_id))),
            SliceKey::Scene { adapter_id, scene_id } => {
                Some((self.scenes.get(usize::from(adapter_id))?, bit(scene_id)))
            }
            SliceKey::InputDevices { bank } => {
                global(bit(bank).checked_shl(FIRST_INPUT_DEVICE_BANK).unwrap_or(0))
            }
            SliceKey::Adapters => global(ADAPTERS),
            SliceKey::HclSchedules => global(HCL_SCHEDULES),
            SliceKey::PollerSettings => global(POLLER_SETTINGS),
            SliceKey::DaliSettings => global(DALI_SETTINGS),
            SliceKey::RedundancySettings => global(REDUNDANCY_SETTINGS),
            SliceKey::Policies => global(POLICIES),
            SliceKey::HomeAssistantSettings => global(HOME_ASSISTANT_SETTINGS),
            SliceKey::PhysicalDevices { .. }
            | SliceKey::PhysicalDeviceBank { .. }
            | SliceKey::ControllerSettings
            | SliceKey::Rules { .. } => None,
        }
    }

    pub(crate) fn global(&self, mask: u32) -> bool {
        self.global.load(Ordering::Acquire) & mask != 0
    }

    pub(crate) fn input_device_banks(&self) -> u8 {
        let banks = self.global.load(Ordering::Acquire) >> FIRST_INPUT_DEVICE_BANK;
        u8::try_from(banks & u32::from(ALL_INPUT_DEVICE_BANKS)).unwrap_or(ALL_INPUT_DEVICE_BANKS)
    }

    pub(crate) fn groups(&self) -> u32 {
        self.groups.load(Ordering::Acquire)
    }

    pub(crate) fn virtual_lamps(&self) -> u32 {
        self.virtual_lamps.load(Ordering::Acquire)
    }

    pub(crate) fn scenes(&self, adapter_id: u8) -> u16 {
        let cell = self.scenes.get(usize::from(adapter_id));
        let mask = cell.map_or(0, |mask| mask.load(Ordering::Acquire));
        u16::try_from(mask).unwrap_or(u16::MAX)
    }

    pub(crate) fn count(&self) -> u32 {
        let cells = [&self.global, &self.groups, &self.virtual_lamps];
        cells
            .into_iter()
            .chain(self.scenes.iter())
            .map(|cell| cell.load(Ordering::Acquire).count_ones())
            .sum()
    }

    pub(crate) fn global_keys(&self, pending: u32) -> impl Iterator<Item = SliceKey> + '_ {
        let withheld = self.global.load(Ordering::Acquire) & pending;
        GLOBALS.into_iter().filter(move |(mask, _)| withheld & mask != 0).map(|(_, key)| key)
    }
}

#[cfg(test)]
mod tests {
    use dali2rust_platform::slice_store::SliceKey;

    use super::{WithheldSlices, ADAPTERS, POLICIES};

    #[test]
    fn a_slice_is_withheld_until_a_read_settles_it() {
        let withheld = WithheldSlices::default();
        withheld.settle(SliceKey::Policies, true);
        withheld.settle(SliceKey::Groups { adapter_id: 1 }, true);
        withheld.settle(SliceKey::Scene { adapter_id: 0, scene_id: 15 }, true);
        withheld.settle(SliceKey::InputDevices { bank: 2 }, true);
        withheld.settle(SliceKey::PhysicalDeviceBank { adapter_id: 0, bank: 0 }, true);
        assert!(withheld.global(POLICIES) && !withheld.global(ADAPTERS));
        assert_eq!(withheld.groups(), 0b10);
        assert_eq!(withheld.scenes(0), 1 << 15);
        assert_eq!(withheld.input_device_banks(), 0b100);
        assert_eq!(withheld.count(), 4);

        withheld.settle(SliceKey::Policies, false);
        withheld.settle(SliceKey::Groups { adapter_id: 1 }, false);
        assert!(!withheld.global(POLICIES));
        assert_eq!(withheld.groups(), 0);
        assert_eq!(withheld.count(), 2);
    }
}
