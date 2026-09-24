use std::sync::Arc;

use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_platform::wall_clock::WallClock;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ControllerSettingsSlice {
    pub version: u32,
    pub timezone: String,
}

const SETTINGS_VERSION: u32 = 1;

pub fn hydrate_timezone(clock: &dyn WallClock, slices: Option<&Arc<dyn SliceStore>>) {
    let Some(store) = slices else {
        return;
    };
    let Ok(bytes) = store.load(SliceKey::ControllerSettings) else {
        return;
    };
    match postcard::from_bytes::<ControllerSettingsSlice>(&bytes) {
        Ok(slice) if slice.version == SETTINGS_VERSION => {
            if let Err(e) = clock.set_timezone(&slice.timezone) {
                log::warn!("time: stored timezone {:?} rejected: {e:?}", slice.timezone);
            }
        }
        Ok(slice) => log::warn!("time: settings slice v{} ignored", slice.version),
        Err(e) => log::warn!("time: settings slice undecodable: {e}"),
    }
}

pub fn timezone_writer(slices: Option<Arc<dyn SliceStore>>) -> Arc<dyn Fn(&str) + Send + Sync> {
    Arc::new(move |timezone: &str| {
        let Some(store) = slices.as_ref() else {
            return;
        };
        let slice = ControllerSettingsSlice {
            version: SETTINGS_VERSION,
            timezone: timezone.to_string(),
        };
        let Ok(bytes) = postcard::to_allocvec(&slice) else {
            log::warn!("time: timezone {timezone:?} did not encode");
            return;
        };
        match store.begin_write(SliceKey::ControllerSettings) {
            Ok(mut session) => {
                if let Err(e) = session.append(&bytes).and_then(|()| session.commit()) {
                    log::warn!("time: timezone persist failed: {e}");
                }
            }
            Err(e) => log::warn!("time: timezone persist could not start: {e}"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_bsp::slice_store_files::InMemorySliceStore;
    use dali2rust_bsp::wall_clock::SystemWallClock;

    #[test]
    fn the_zone_round_trips_through_the_store() {
        let store: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
        timezone_writer(Some(Arc::clone(&store)))("MSK-3");

        let clock = SystemWallClock::new();
        assert_eq!(clock.timezone(), "UTC0");
        hydrate_timezone(&clock, Some(&store));
        assert_eq!(clock.timezone(), "MSK-3");
    }

    #[test]
    fn a_controller_without_persistence_keeps_its_default() {
        let clock = SystemWallClock::new();
        hydrate_timezone(&clock, None);
        assert_eq!(clock.timezone(), "UTC0");
    }
}
