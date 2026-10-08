use std::sync::Arc;

use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_api::http::handlers::time::{TimezonePersist, TimezonePersistRefusal};
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

pub fn timezone_writer(slices: Option<Arc<dyn SliceStore>>) -> TimezonePersist {
    Arc::new(move |timezone: &str| {
        let Some(store) = slices.as_ref() else {
            return Ok(());
        };
        if !dali2rust_platform::flash_gate::writable_now() {
            return Err(TimezonePersistRefusal::FlashBusy);
        }
        let slice = ControllerSettingsSlice {
            version: SETTINGS_VERSION,
            timezone: timezone.to_string(),
        };
        let Ok(bytes) = postcard::to_allocvec(&slice) else {
            log::warn!("time: timezone {timezone:?} did not encode");
            return Err(TimezonePersistRefusal::StoreFailed);
        };
        let written = store
            .begin_write(SliceKey::ControllerSettings)
            .and_then(|mut session| session.append(&bytes).and_then(|()| session.commit()));
        written.map_err(|e| {
            log::warn!("time: timezone persist failed: {e}");
            TimezonePersistRefusal::StoreFailed
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_bsp::slice_store_files::InMemorySliceStore;
    use dali2rust_bsp::wall_clock::SystemWallClock;
    use dali2rust_api::http::handler::ApiHandler;

    #[test]
    fn the_zone_round_trips_through_the_store() {
        let store: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
        timezone_writer(Some(Arc::clone(&store)))("MSK-3").expect("the gate is free");

        let clock = SystemWallClock::new();
        assert_eq!(clock.timezone(), "UTC0");
        hydrate_timezone(&clock, Some(&store));
        assert_eq!(clock.timezone(), "MSK-3");
    }

    #[test]
    fn a_zone_the_flash_cannot_take_now_is_refused_and_not_applied() {
        let clock: Arc<dyn WallClock> = Arc::new(SystemWallClock::new());
        let busy: TimezonePersist = Arc::new(|_| Err(TimezonePersistRefusal::FlashBusy));
        let handler =
            dali2rust_api::http::handlers::time::TimeHandler::new(Arc::clone(&clock), busy);
        let response = handler.handle_request(
            "PUT",
            "/api/v1/time",
            br#"{"timezone":"MSK-3"}"#,
            &std::collections::HashMap::new(),
        );
        assert_eq!(response.status, 503);
        assert_eq!(clock.timezone(), "UTC0", "a refused zone must not be left in force");
    }

    fn put_zone(clock: &Arc<dyn WallClock>, persist: TimezonePersist, zone: &str) -> u16 {
        let handler = dali2rust_api::http::handlers::time::TimeHandler::new(Arc::clone(clock), persist);
        let body = format!(r#"{{"timezone":"{zone}"}}"#);
        handler
            .handle_request("PUT", "/api/v1/time", body.as_bytes(), &std::collections::HashMap::new())
            .status
    }

    #[test]
    fn a_zone_the_store_failed_to_write_is_refused_and_not_applied() {
        let clock: Arc<dyn WallClock> = Arc::new(SystemWallClock::new());
        let failing: TimezonePersist = Arc::new(|_| Err(TimezonePersistRefusal::StoreFailed));
        assert_eq!(put_zone(&clock, failing, "MSK-3"), 503);
        assert_eq!(clock.timezone(), "UTC0");
    }

    #[test]
    fn the_zone_already_in_force_does_not_touch_the_flash() {
        let clock: Arc<dyn WallClock> = Arc::new(SystemWallClock::new());
        let writes = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counted = Arc::clone(&writes);
        let persist: TimezonePersist = Arc::new(move |_| {
            counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        });
        assert_eq!(put_zone(&clock, Arc::clone(&persist), "MSK-3"), 200);
        assert_eq!(put_zone(&clock, persist, "MSK-3"), 200);
        assert_eq!(writes.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn a_controller_without_persistence_keeps_its_default() {
        let clock = SystemWallClock::new();
        hydrate_timezone(&clock, None);
        assert_eq!(clock.timezone(), "UTC0");
    }
}
