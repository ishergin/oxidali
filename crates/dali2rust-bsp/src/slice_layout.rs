use dali2rust_platform::slice_store::SliceKey;

pub const SECTOR_BYTES: u32 = 4096;

pub const MAX_ADAPTERS: u32 = 8;

pub const SCENES_PER_ADAPTER: u32 = 16;

const LARGE_BANK_SECTORS: u32 = 8;

const SMALL_BANK_SECTORS: u32 = 1;

const SMALL_SLOTS_PER_ADAPTER: u32 = 2 + SCENES_PER_ADAPTER;

const LARGE_SLOT_COUNT: u32 = MAX_ADAPTERS;

const PD_BANKS_PER_ADAPTER: u32 = 16;

const PD_BANK_SECTORS: u32 = 1;
const PD_BANK_BYTES: u32 = PD_BANK_SECTORS * SECTOR_BYTES;
const PD_BANK_REGION_BYTES: u32 = MAX_ADAPTERS * PD_BANKS_PER_ADAPTER * 2 * PD_BANK_BYTES;

const LEADING_GLOBAL_SMALL_SLOTS: u32 = 2;
const TRAILING_GLOBAL_SMALL_SLOTS: u32 = 14;
const TRAILING_GLOBAL_BASE: u32 = LEADING_GLOBAL_SMALL_SLOTS + MAX_ADAPTERS * SMALL_SLOTS_PER_ADAPTER;
const SMALL_SLOT_COUNT: u32 = TRAILING_GLOBAL_BASE + TRAILING_GLOBAL_SMALL_SLOTS;

const LARGE_BANK_BYTES: u32 = LARGE_BANK_SECTORS * SECTOR_BYTES;
const SMALL_BANK_BYTES: u32 = SMALL_BANK_SECTORS * SECTOR_BYTES;

const LARGE_REGION_BYTES: u32 = LARGE_SLOT_COUNT * 2 * LARGE_BANK_BYTES;

pub const LAYOUT_BYTES: u32 =
    LARGE_REGION_BYTES + SMALL_SLOT_COUNT * 2 * SMALL_BANK_BYTES + PD_BANK_REGION_BYTES;

pub const BANK_HEADER_BYTES: u32 = 16;

pub const SMALL_SLOT_PAYLOAD_BYTES: u32 = SMALL_BANK_BYTES - BANK_HEADER_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotGeometry {
    pub offset: u32,
    pub bank_bytes: u32,
}

impl SlotGeometry {
    pub fn bank_offset(&self, bank: u8) -> u32 {
        self.offset + u32::from(bank) * self.bank_bytes
    }
}

pub fn slot_geometry(key: SliceKey) -> Option<SlotGeometry> {
    match key {
        SliceKey::PhysicalDevices { adapter_id } => {
            let index = u32::from(adapter_id);
            (index < MAX_ADAPTERS).then(|| SlotGeometry {
                offset: index * 2 * LARGE_BANK_BYTES,
                bank_bytes: LARGE_BANK_BYTES,
            })
        }
        SliceKey::PhysicalDeviceBank { adapter_id, bank } => {
            let adapter = u32::from(adapter_id);
            let bank = u32::from(bank);
            (adapter < MAX_ADAPTERS && bank < PD_BANKS_PER_ADAPTER).then(|| {
                let index = adapter * PD_BANKS_PER_ADAPTER + bank;
                SlotGeometry {
                    offset: LARGE_REGION_BYTES
                        + SMALL_SLOT_COUNT * 2 * SMALL_BANK_BYTES
                        + index * 2 * PD_BANK_BYTES,
                    bank_bytes: PD_BANK_BYTES,
                }
            })
        }
        _ => small_slot_index(key).map(|index| SlotGeometry {
            offset: LARGE_REGION_BYTES + index * 2 * SMALL_BANK_BYTES,
            bank_bytes: SMALL_BANK_BYTES,
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankChoice {
    pub keep: Option<u8>,
    pub target: u8,
}

pub fn choose_bank(proved: Option<u8>, newest_header: Option<u8>) -> BankChoice {
    let keep = proved.or(newest_header);
    BankChoice {
        keep,
        target: keep.map_or(0, |bank| 1 - bank),
    }
}

pub const PD_BANK_SLOT_COUNT: u32 = MAX_ADAPTERS * PD_BANKS_PER_ADAPTER;
pub const TOTAL_SLOT_COUNT: u32 = LARGE_SLOT_COUNT + SMALL_SLOT_COUNT + PD_BANK_SLOT_COUNT;

pub fn slot_index(key: SliceKey) -> Option<u32> {
    match key {
        SliceKey::PhysicalDevices { adapter_id } => {
            let index = u32::from(adapter_id);
            (index < MAX_ADAPTERS).then_some(index)
        }
        SliceKey::PhysicalDeviceBank { adapter_id, bank } => {
            let adapter = u32::from(adapter_id);
            let bank = u32::from(bank);
            (adapter < MAX_ADAPTERS && bank < PD_BANKS_PER_ADAPTER).then(|| {
                LARGE_SLOT_COUNT + SMALL_SLOT_COUNT + adapter * PD_BANKS_PER_ADAPTER + bank
            })
        }
        _ => small_slot_index(key).map(|index| LARGE_SLOT_COUNT + index),
    }
}

pub const INPUT_DEVICE_BANKS: u32 = 4;
pub const RULES_BANKS: u32 = 4;

const _: () = assert!(INPUT_DEVICE_BANKS == dali2rust_platform::slice_store::SliceKey::INPUT_DEVICE_BANKS as u32);
const _: () = assert!(RULES_BANKS == dali2rust_platform::slice_store::SliceKey::RULES_BANKS as u32);

fn global_slot_index(key: SliceKey) -> Option<Option<u32>> {
    let slot = match key {
        SliceKey::Adapters => Some(0),
        SliceKey::ControllerSettings => Some(1),
        SliceKey::HclSchedules => Some(TRAILING_GLOBAL_BASE),
        SliceKey::PollerSettings => Some(TRAILING_GLOBAL_BASE + 1),
        SliceKey::HomeAssistantSettings => Some(TRAILING_GLOBAL_BASE + 2),
        SliceKey::DaliSettings => Some(TRAILING_GLOBAL_BASE + 3),
        SliceKey::InputDevices { bank } => (u32::from(bank) < INPUT_DEVICE_BANKS)
            .then(|| TRAILING_GLOBAL_BASE + 4 + u32::from(bank)),
        SliceKey::Rules { bank } => (u32::from(bank) < RULES_BANKS)
            .then(|| TRAILING_GLOBAL_BASE + 4 + INPUT_DEVICE_BANKS + u32::from(bank)),
        SliceKey::RedundancySettings => {
            Some(TRAILING_GLOBAL_BASE + 4 + INPUT_DEVICE_BANKS + RULES_BANKS)
        }
        SliceKey::Policies => Some(TRAILING_GLOBAL_BASE + 5 + INPUT_DEVICE_BANKS + RULES_BANKS),
        _ => return None,
    };
    Some(slot)
}

fn bounded(index: u32) -> Option<u32> {
    (index < SMALL_SLOT_COUNT).then_some(index)
}

fn small_slot_index(key: SliceKey) -> Option<u32> {
    if let Some(slot) = global_slot_index(key) {
        return slot.and_then(bounded);
    }
    let (adapter_id, within_adapter) = match key {
        SliceKey::VirtualLamps { adapter_id } => (adapter_id, 0),
        SliceKey::Groups { adapter_id } => (adapter_id, 1),
        SliceKey::Scene {
            adapter_id,
            scene_id,
        } => {
            if u32::from(scene_id) >= SCENES_PER_ADAPTER {
                return None;
            }
            (adapter_id, 2 + u32::from(scene_id))
        }
        _ => return None,
    };
    let adapter_index = u32::from(adapter_id);
    if adapter_index >= MAX_ADAPTERS {
        return None;
    }
    Some(LEADING_GLOBAL_SMALL_SLOTS + adapter_index * SMALL_SLOTS_PER_ADAPTER + within_adapter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn exhaustive(key: SliceKey) {
        match key {
            SliceKey::Adapters
            | SliceKey::ControllerSettings
            | SliceKey::HclSchedules
            | SliceKey::PollerSettings
            | SliceKey::HomeAssistantSettings
            | SliceKey::DaliSettings
            | SliceKey::RedundancySettings
            | SliceKey::Policies
            | SliceKey::InputDevices { .. }
            | SliceKey::Rules { .. }
            | SliceKey::VirtualLamps { .. }
            | SliceKey::Groups { .. }
            | SliceKey::PhysicalDevices { .. }
            | SliceKey::PhysicalDeviceBank { .. }
            | SliceKey::Scene { .. } => {}
        }
    }

    fn every_key() -> Vec<SliceKey> {
        exhaustive(SliceKey::Adapters);
        let mut keys = vec![
            SliceKey::Adapters,
            SliceKey::ControllerSettings,
            SliceKey::HclSchedules,
            SliceKey::PollerSettings,
            SliceKey::HomeAssistantSettings,
            SliceKey::DaliSettings,
            SliceKey::RedundancySettings,
            SliceKey::Policies,
            SliceKey::InputDevices { bank: 0 },
            SliceKey::InputDevices { bank: 1 },
            SliceKey::InputDevices { bank: 2 },
            SliceKey::InputDevices { bank: 3 },
            SliceKey::Rules { bank: 0 },
            SliceKey::Rules { bank: 1 },
            SliceKey::Rules { bank: 2 },
            SliceKey::Rules { bank: 3 },
        ];
        for adapter_id in 0..MAX_ADAPTERS as u8 {
            keys.push(SliceKey::VirtualLamps { adapter_id });
            keys.push(SliceKey::Groups { adapter_id });
            keys.push(SliceKey::PhysicalDevices { adapter_id });
            for bank in 0..PD_BANKS_PER_ADAPTER as u8 {
                keys.push(SliceKey::PhysicalDeviceBank { adapter_id, bank });
            }
            for scene_id in 0..SCENES_PER_ADAPTER as u8 {
                keys.push(SliceKey::Scene {
                    adapter_id,
                    scene_id,
                });
            }
        }
        keys
    }

    #[test]
    fn no_two_slices_share_flash() {
        let mut seen: HashSet<u32> = HashSet::new();
        for key in every_key() {
            let slot = slot_geometry(key).expect("reserved key");
            for bank in 0..2u8 {
                let start = slot.bank_offset(bank);
                for sector in (start..start + slot.bank_bytes).step_by(SECTOR_BYTES as usize) {
                    assert!(seen.insert(sector), "sector {sector} reused by {key:?}");
                }
            }
        }
        assert_eq!(seen.len() as u32, LAYOUT_BYTES / SECTOR_BYTES);
    }

    #[test]
    fn every_bank_is_sector_aligned() {
        for key in every_key() {
            let slot = slot_geometry(key).expect("reserved key");
            for bank in 0..2u8 {
                assert_eq!(slot.bank_offset(bank) % SECTOR_BYTES, 0, "{key:?}");
            }
        }
    }

    #[test]
    fn an_index_past_the_reservation_has_no_slot() {
        assert_eq!(bounded(SMALL_SLOT_COUNT - 1), Some(SMALL_SLOT_COUNT - 1));
        assert_eq!(bounded(SMALL_SLOT_COUNT), None);
    }

    #[test]
    fn every_global_slice_has_a_slot() {
        for key in every_key() {
            assert!(slot_geometry(key).is_some(), "no slot for {key:?}");
        }
    }

    #[test]
    fn out_of_range_keys_have_no_slot() {
        assert!(slot_geometry(SliceKey::Groups { adapter_id: 8 }).is_none());
        assert!(slot_geometry(SliceKey::Scene {
            adapter_id: 0,
            scene_id: 16
        })
        .is_none());
    }

    #[test]
    fn layout_fits_the_partition() {
        const { assert!(LAYOUT_BYTES < 4 * 1024 * 1024, "the slice layout must fit the partition") };
    }

    #[test]
    fn every_key_has_its_own_slot_index_inside_the_total() {
        let mut seen = HashSet::new();
        for key in every_key() {
            let index = slot_index(key).unwrap_or_else(|| panic!("{key:?} has no slot index"));
            assert!(
                index < TOTAL_SLOT_COUNT,
                "{key:?} indexes {index}, past TOTAL_SLOT_COUNT {TOTAL_SLOT_COUNT}"
            );
            assert!(seen.insert(index), "{key:?} shares slot index {index}");
        }
        assert_eq!(
            seen.len() as u32,
            TOTAL_SLOT_COUNT,
            "TOTAL_SLOT_COUNT must be exactly the slots that exist"
        );
    }

    #[test]
    fn a_torn_newer_header_never_costs_the_bank_that_load_proved() {
        let choice = choose_bank(Some(0), Some(1));
        assert_eq!(choice.keep, Some(0), "the proved payload is what must survive");
        assert_eq!(
            choice.target, 1,
            "targeting bank 0 erases the only readable copy (ISSUE-56 step 6)"
        );

        let mirrored = choose_bank(Some(1), Some(0));
        assert_eq!(mirrored.keep, Some(1));
        assert_eq!(mirrored.target, 0);
    }

    #[test]
    fn the_header_decides_only_when_nothing_was_proved() {
        assert_eq!(
            choose_bank(None, Some(1)),
            BankChoice {
                keep: Some(1),
                target: 0
            }
        );
        assert_eq!(
            choose_bank(None, None),
            BankChoice {
                keep: None,
                target: 0
            },
            "an empty slot has nothing to keep"
        );
    }

    #[test]
    fn agreement_alternates_banks() {
        for bank in 0..2u8 {
            let choice = choose_bank(Some(bank), Some(bank));
            assert_eq!(choice.keep, Some(bank));
            assert_eq!(choice.target, 1 - bank);
        }
    }
}
