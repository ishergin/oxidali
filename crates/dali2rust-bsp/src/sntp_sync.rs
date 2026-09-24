use std::sync::Arc;

use crate::wall_clock::SystemWallClock;
use esp_idf_svc::sntp::{EspSntp, SyncStatus};

const WATCH_PERIOD: std::time::Duration = std::time::Duration::from_secs(5);

pub fn start(clock: Arc<SystemWallClock>) {
    let sntp = match EspSntp::new_default() {
        Ok(sntp) => sntp,
        Err(e) => {
            log::warn!("sntp: unavailable ({e:?}) — the clock stays on the manual path");
            return;
        }
    };
    // SAFETY: the SNTP client must live for the process lifetime; dropping it stops the periodic re-sync.
    let sntp: &'static EspSntp = Box::leak(Box::new(sntp));

    crate::esp_thread::spawn_named_stack_in(
        c"sntp-watch",
        crate::std_thread_stack::DISPLAY_WORKER,
        crate::esp_thread::StackHome::ExternalOnXip,
        move || watch_until_synced(sntp, clock),
    );
}

fn watch_until_synced(sntp: &'static EspSntp, clock: Arc<SystemWallClock>) {
    loop {
        if sntp.get_sync_status() == SyncStatus::Completed
            || SystemWallClock::system_clock_is_plausible()
        {
            clock.adopt_sntp();
            log::info!("sntp: clock synchronized");
            return;
        }
        // sleep-ok: 5 s idle back-off in a one-shot watcher, far above the 10 ms floor.
        std::thread::sleep(WATCH_PERIOD);
    }
}
