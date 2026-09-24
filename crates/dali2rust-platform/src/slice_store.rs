#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SliceKey {
    Adapters,
    VirtualLamps { adapter_id: u8 },
    Groups { adapter_id: u8 },
    PhysicalDevices { adapter_id: u8 },
    Scene { adapter_id: u8, scene_id: u8 },
    ControllerSettings,
    HclSchedules,
    PollerSettings,
    InputDevices { bank: u8 },
    Rules { bank: u8 },
    HomeAssistantSettings,
    DaliSettings,
    RedundancySettings,
    Policies,
    PhysicalDeviceBank { adapter_id: u8, bank: u8 },
}

impl SliceKey {
    pub const SCENES_PER_ADAPTER: u8 = 16;
    pub const INPUT_DEVICE_BANKS: u8 = 4;
    pub const RULES_BANKS: u8 = 4;
    pub const PHYSICAL_DEVICE_BANKS: u8 = 16;
    pub const DEVICES_PER_BANK: u8 = 4;

    pub fn every(adapter_count: u8) -> impl Iterator<Item = SliceKey> {
        let per_adapter = (0..adapter_count).flat_map(|adapter_id| {
            [
                SliceKey::VirtualLamps { adapter_id },
                SliceKey::Groups { adapter_id },
                SliceKey::PhysicalDevices { adapter_id },
            ]
            .into_iter()
            .chain(
                (0..Self::PHYSICAL_DEVICE_BANKS)
                    .map(move |bank| SliceKey::PhysicalDeviceBank { adapter_id, bank }),
            )
            .chain((0..Self::SCENES_PER_ADAPTER).map(move |scene_id| SliceKey::Scene {
                adapter_id,
                scene_id,
            }))
        });
        core::iter::once(SliceKey::Adapters)
            .chain(per_adapter)
            .chain([SliceKey::ControllerSettings, SliceKey::HclSchedules, SliceKey::PollerSettings])
            .chain((0..Self::INPUT_DEVICE_BANKS).map(|bank| SliceKey::InputDevices { bank }))
            .chain((0..Self::RULES_BANKS).map(|bank| SliceKey::Rules { bank }))
            .chain([
                SliceKey::HomeAssistantSettings,
                SliceKey::DaliSettings,
                SliceKey::RedundancySettings,
                SliceKey::Policies,
            ])
    }

    pub fn label(&self) -> String {
        match self {
            SliceKey::Adapters => "adapters".to_string(),
            SliceKey::VirtualLamps { adapter_id } => format!("a{adapter_id}/virtual_lamps"),
            SliceKey::Groups { adapter_id } => format!("a{adapter_id}/groups"),
            SliceKey::PhysicalDevices { adapter_id } => format!("a{adapter_id}/physical_devices"),
            SliceKey::Scene {
                adapter_id,
                scene_id,
            } => format!("a{adapter_id}/scene{scene_id}"),
            SliceKey::ControllerSettings => "settings".to_string(),
            SliceKey::HclSchedules => "hcl_schedules".to_string(),
            SliceKey::PollerSettings => "poller_settings".to_string(),
            SliceKey::InputDevices { bank } => format!("input_devices_b{bank}"),
            SliceKey::Rules { bank } => format!("rules_b{bank}"),
            SliceKey::HomeAssistantSettings => "home_assistant_settings".to_string(),
            SliceKey::DaliSettings => "dali_settings".to_string(),
            SliceKey::RedundancySettings => "redundancy_settings".to_string(),
            SliceKey::Policies => "policies".to_string(),
            SliceKey::PhysicalDeviceBank { adapter_id, bank } => {
                format!("a{adapter_id}/physical_devices_b{bank}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum StoreError {
    Missing,
    TooLarge { len: usize, capacity: usize },
    Backend(String),
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StoreError::Missing => write!(f, "missing"),
            StoreError::TooLarge { len, capacity } => {
                write!(f, "slice of {len} B exceeds slot capacity {capacity} B")
            }
            StoreError::Backend(msg) => write!(f, "{msg}"),
        }
    }
}

pub trait SliceWriteSession {
    fn append(&mut self, chunk: &[u8]) -> Result<(), StoreError>;

    fn commit(self: Box<Self>) -> Result<(), StoreError>;

    fn abort(self: Box<Self>);
}

pub trait SliceStore: Send + Sync {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError>;

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError>;
}

pub struct InMemorySlices {
    entries: Vec<(SliceKey, Result<Vec<u8>, StoreError>)>,
}

impl InMemorySlices {
    pub fn snapshot(store: &dyn SliceStore, adapter_count: u8) -> Self {
        Self {
            entries: SliceKey::every(adapter_count)
                .map(|key| (key, store.load(key)))
                .collect(),
        }
    }

    #[must_use]
    pub fn bytes(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|(_, r)| r.as_ref().ok().map(Vec::len))
            .sum()
    }
}

impl SliceStore for InMemorySlices {
    fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
        self.entries
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(Err(StoreError::Missing), |(_, r)| r.clone())
    }

    fn begin_write(&self, key: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
        Err(StoreError::Backend(format!("{}: snapshot is read-only", key.label())))
    }
}

#[cfg(test)]
mod tests {
    use super::{InMemorySlices, SliceKey, SliceStore, SliceWriteSession, StoreError};
    use std::collections::HashSet;

    #[test]
    fn every_variant_is_enumerated() {
        let keys: HashSet<SliceKey> = SliceKey::every(2).collect();
        let witness = |key: SliceKey| match key {
            SliceKey::Adapters
            | SliceKey::VirtualLamps { .. }
            | SliceKey::Groups { .. }
            | SliceKey::PhysicalDevices { .. }
            | SliceKey::Scene { .. }
            | SliceKey::ControllerSettings
            | SliceKey::HclSchedules
            | SliceKey::PollerSettings
            | SliceKey::InputDevices { .. }
            | SliceKey::Rules { .. }
            | SliceKey::HomeAssistantSettings
            | SliceKey::DaliSettings
            | SliceKey::RedundancySettings
            | SliceKey::Policies
            | SliceKey::PhysicalDeviceBank { .. } => keys.contains(&key),
        };
        for key in [
            SliceKey::Adapters,
            SliceKey::VirtualLamps { adapter_id: 1 },
            SliceKey::Groups { adapter_id: 1 },
            SliceKey::PhysicalDevices { adapter_id: 1 },
            SliceKey::PhysicalDeviceBank { adapter_id: 1, bank: 0 },
            SliceKey::PhysicalDeviceBank { adapter_id: 1, bank: 7 },
            SliceKey::Scene { adapter_id: 1, scene_id: 15 },
            SliceKey::ControllerSettings,
            SliceKey::HclSchedules,
            SliceKey::PollerSettings,
            SliceKey::InputDevices { bank: 3 },
            SliceKey::Rules { bank: 3 },
            SliceKey::HomeAssistantSettings,
            SliceKey::DaliSettings,
            SliceKey::RedundancySettings,
            SliceKey::Policies,
        ] {
            assert!(witness(key), "{key:?} is not enumerated");
        }
        assert_eq!(keys.len(), 86);
        assert_eq!(SliceKey::every(2).count(), 86);
        assert!(!keys.contains(&SliceKey::Scene { adapter_id: 2, scene_id: 0 }));
    }

    struct Two;
    impl SliceStore for Two {
        fn load(&self, key: SliceKey) -> Result<Vec<u8>, StoreError> {
            match key {
                SliceKey::Policies => Ok(vec![1, 2, 3]),
                SliceKey::DaliSettings => Err(StoreError::Backend("torn".into())),
                _ => Err(StoreError::Missing),
            }
        }
        fn begin_write(&self, _: SliceKey) -> Result<Box<dyn SliceWriteSession + '_>, StoreError> {
            Err(StoreError::Missing)
        }
    }

    #[test]
    fn a_snapshot_answers_as_the_store_did_and_refuses_writes() {
        let snap = InMemorySlices::snapshot(&Two, 1);
        assert_eq!(snap.load(SliceKey::Policies).expect("bytes"), vec![1, 2, 3]);
        assert!(matches!(snap.load(SliceKey::HclSchedules), Err(StoreError::Missing)));
        assert!(matches!(snap.load(SliceKey::DaliSettings), Err(StoreError::Backend(_))));
        assert_eq!(snap.bytes(), 3);
        assert!(snap.begin_write(SliceKey::Policies).is_err());
    }

    #[test]
    fn labels_are_distinct_and_stable() {
        assert_eq!(SliceKey::Adapters.label(), "adapters");
        assert_eq!(
            SliceKey::Scene {
                adapter_id: 0,
                scene_id: 3
            }
            .label(),
            "a0/scene3"
        );
        assert_eq!(
            SliceKey::PhysicalDevices { adapter_id: 1 }.label(),
            "a1/physical_devices"
        );
    }
}
