mod support;

use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::BusConfig;
use dali2rust_display_runtime::{DisplaySample, DisplayView};
use dali2rust_test_support::wait_until;

use support::{healthy_sample, spawn_display_worker_on_moving_source, MovingDisplaySource};

fn row_says(view: &DisplayView, row: usize, needle: &str) -> bool {
    view.lines()[row].text.contains(needle)
}

fn observe_a_sample(view: &DisplayView, source: &MovingDisplaySource, marker: u32) {
    source.update(|s| s.frames_sent_total = marker);
    wait_until(
        || row_says(view, 2, &format!("{marker}F/M")),
        Duration::from_secs(4),
    );
}

#[test]
fn a_read_failure_holds_the_poller_row_rather_than_flashing_it() {
    let source = Arc::new(MovingDisplaySource::new(DisplaySample {
        poller_enabled: true,
        poller_interval_ms: 5_000,
        ..healthy_sample()
    }));
    let (_host, _publisher, view, _w) =
        spawn_display_worker_on_moving_source(BusConfig::default(), Arc::clone(&source));
    wait_until(|| row_says(&view, 4, "POLL 5S OK"), Duration::from_secs(3));

    source.update(|s| s.poller_reads_failed = 1);
    wait_until(|| row_says(&view, 4, "POLL 5S ER"), Duration::from_secs(3));

    observe_a_sample(&view, &source, 7);
    observe_a_sample(&view, &source, 11);
    assert!(
        row_says(&view, 4, "POLL 5S ER"),
        "the flag must outlive the tick that saw the counter move: {:?}",
        view.lines()[4].text
    );
}

#[test]
fn absent_reads_reach_the_poller_row_on_their_own_counter() {
    let source = Arc::new(MovingDisplaySource::new(DisplaySample {
        poller_enabled: true,
        poller_interval_ms: 5_000,
        ..healthy_sample()
    }));
    let (_host, _publisher, view, _w) =
        spawn_display_worker_on_moving_source(BusConfig::default(), Arc::clone(&source));
    wait_until(|| row_says(&view, 4, "POLL 5S OK"), Duration::from_secs(3));

    source.update(|s| s.poller_reads_absent = 28);
    wait_until(|| row_says(&view, 4, "POLL 5S NA"), Duration::from_secs(3));
}

#[test]
fn a_persistence_failure_reaches_the_services_row() {
    let source = Arc::new(MovingDisplaySource::new(DisplaySample {
        poller_enabled: true,
        poller_interval_ms: 5_000,
        ..healthy_sample()
    }));
    let (_host, _publisher, view, _w) =
        spawn_display_worker_on_moving_source(BusConfig::default(), Arc::clone(&source));
    wait_until(|| row_says(&view, 4, "POLL 5S OK"), Duration::from_secs(3));

    source.update(|s| s.persistence_failed = 2);
    wait_until(|| row_says(&view, 4, "NO SAVE"), Duration::from_secs(3));
    assert!(view.lines()[4].invert, "a lost setting takes the alarm treatment");
}

#[test]
fn the_bus_row_reports_frames_that_actually_moved() {
    let source = Arc::new(MovingDisplaySource::new(healthy_sample()));
    let (_host, _publisher, view, _w) =
        spawn_display_worker_on_moving_source(BusConfig::default(), Arc::clone(&source));
    wait_until(|| row_says(&view, 2, "0F/M"), Duration::from_secs(3));

    source.update(|s| {
        s.frames_sent_total = 150;
        s.foreign_frames_total = 12;
    });
    wait_until(|| row_says(&view, 2, "150F/M F12"), Duration::from_secs(4));
}

#[test]
fn the_bus_row_histogram_fills_from_the_right() {
    let source = Arc::new(MovingDisplaySource::new(DisplaySample {
        wire_load_permille: 750,
        ..healthy_sample()
    }));
    let (_host, _publisher, view, _w) =
        spawn_display_worker_on_moving_source(BusConfig::default(), Arc::clone(&source));
    let last = |v: &DisplayView, col: usize| v.snapshot().load_history[col];
    let newest = dali2rust_display_runtime::SPARK_COLS - 1;
    assert!(view.lines()[2].spark, "the bus row lends its cells to the histogram");
    wait_until(|| last(&view, newest) > 0, Duration::from_secs(4));
    assert_eq!(last(&view, newest - 1), 0, "history must not fill from the left");
    wait_until(|| last(&view, newest - 1) > 0, Duration::from_secs(8));
    assert!(last(&view, newest) > 0);
}
