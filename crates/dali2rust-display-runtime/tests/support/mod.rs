#![allow(dead_code, reason = "Each test binary compiles this module separately and uses the part it needs: `sampled_deltas` wants the moving source, the others the static one, and neither is dead code from the crate's point of view")]

use std::sync::{Arc, Mutex};

use dali2rust_bus::{BusConfig, BusHost, BusPublisher};
use dali2rust_display_runtime::{
    spawn_display_worker, DisplaySample, DisplaySource, DisplayView, HardwareDisplay,
    StaticDisplaySource, DISPLAY_WORKER_HANDLED_EVENTS,
};

pub fn spawn_display_worker_on_bus(
    config: BusConfig,
    sample: DisplaySample,
) -> (
    BusHost,
    BusPublisher,
    Arc<DisplayView>,
    std::thread::JoinHandle<()>,
) {
    let (host, publisher, ev_rx) = BusHost::spawn(config, |reg| {
        reg.subscribe_events(32, DISPLAY_WORKER_HANDLED_EVENTS)
    });
    let view = Arc::new(DisplayView::default());
    let worker = spawn_display_worker(
        ev_rx,
        Arc::new(StaticDisplaySource(sample)),
        Arc::clone(&view),
        HardwareDisplay::none(),
    );
    (host, publisher, view, worker)
}

#[derive(Default)]
pub struct MovingDisplaySource(Mutex<DisplaySample>);

impl MovingDisplaySource {
    pub fn new(sample: DisplaySample) -> Self {
        Self(Mutex::new(sample))
    }

    pub fn update(&self, f: impl FnOnce(&mut DisplaySample)) {
        let mut g = self.0.lock().expect("sample");
        f(&mut g);
    }
}

impl DisplaySource for MovingDisplaySource {
    fn sample(&self) -> DisplaySample {
        *self.0.lock().expect("sample")
    }
}

pub fn spawn_display_worker_on_moving_source(
    config: BusConfig,
    source: Arc<MovingDisplaySource>,
) -> (
    BusHost,
    BusPublisher,
    Arc<DisplayView>,
    std::thread::JoinHandle<()>,
) {
    let (host, publisher, ev_rx) = BusHost::spawn(config, |reg| {
        reg.subscribe_events(32, DISPLAY_WORKER_HANDLED_EVENTS)
    });
    let view = Arc::new(DisplayView::default());
    let worker = spawn_display_worker(ev_rx, source, Arc::clone(&view), HardwareDisplay::none());
    (host, publisher, view, worker)
}

pub fn healthy_sample() -> DisplaySample {
    DisplaySample {
        gear_known: 12,
        input_known: 3,
        input_present: 3,
        adapter_enabled: true,
        time_synced: true,
        ..DisplaySample::default()
    }
}
