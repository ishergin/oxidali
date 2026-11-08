mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusFrame, BusPublisher, PublishResult};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::{DisplayPage, DisplaySettingsChangedEvent, Origin};
use dali2rust_display_runtime::{RecordingPanel, PAGE_DWELL};
use dali2rust_domain::registry::DisplaySettingsView;
use dali2rust_test_support::{try_wait_until, wait_until};

use support::{healthy_sample, spawn_display_worker_on_panel, MovingDisplaySource};

const REFRESH_WAIT: Duration = Duration::from_secs(3);
const IDLE_PROOF: Duration = Duration::from_millis(2500);

fn announce(publisher: &BusPublisher, settings: DisplaySettingsView) {
    let body = DisplaySettingsChangedEvent {
        enabled: settings.enabled,
        i2c_address: settings.i2c_address,
        upside_down: settings.upside_down,
        screensaver_timeout_s: settings.screensaver_timeout_s,
        pages_mask: settings.pages_mask,
    };
    let ev = event_envelope(0, 1, 0, Some(Origin::Internal), body);
    assert_eq!(
        publisher.try_publish(BusChannel::Events, BusFrame::event(ev)),
        PublishResult::Queued
    );
}

fn with_display(display: DisplaySettingsView) -> Arc<MovingDisplaySource> {
    Arc::new(MovingDisplaySource::new(dali2rust_display_runtime::DisplaySample {
        display,
        ..healthy_sample()
    }))
}

fn powered(panel: &Arc<Mutex<RecordingPanel>>) -> bool {
    panel.lock().expect("panel").powered
}

#[test]
fn a_disabled_display_wakes_only_for_the_settings_event() {
    let off = DisplaySettingsView { enabled: false, ..DisplaySettingsView::default() };
    let source = with_display(off);
    let panel = Arc::new(Mutex::new(RecordingPanel::default()));
    let (_host, publisher, _view, _w) = spawn_display_worker_on_panel(source.clone(), panel.clone());
    wait_until(|| panel.lock().expect("panel").configured == 1, REFRESH_WAIT);
    assert!(!powered(&panel), "a disabled display stays dark");
    source.update(|sample| sample.display.enabled = true);
    let woke = try_wait_until(|| powered(&panel), IDLE_PROOF);
    assert!(!woke, "an idle worker does not poll its settings");
    announce(&publisher, DisplaySettingsView::default());
    wait_until(|| powered(&panel), REFRESH_WAIT);
    assert!(panel.lock().expect("panel").shows_row_starting_with("GEAR "));
}

#[test]
fn a_running_display_picks_up_a_new_rotation_without_the_event() {
    let source = with_display(DisplaySettingsView::default());
    let panel = Arc::new(Mutex::new(RecordingPanel::default()));
    let (_host, _publisher, _view, _w) = spawn_display_worker_on_panel(source.clone(), panel.clone());
    wait_until(|| powered(&panel), REFRESH_WAIT);
    source.update(|sample| sample.display.upside_down = true);
    wait_until(
        || panel.lock().expect("panel").setup.is_some_and(|setup| setup.upside_down),
        REFRESH_WAIT,
    );
    wait_until(|| powered(&panel), REFRESH_WAIT);
}

#[test]
fn the_selected_pages_take_turns_on_the_panel() {
    let both = DisplaySettingsView {
        pages_mask: DisplayPage::Summary.mask_bit() | DisplayPage::Bus.mask_bit(),
        ..DisplaySettingsView::default()
    };
    let source = with_display(both);
    let panel = Arc::new(Mutex::new(RecordingPanel::default()));
    let (_host, _publisher, view, _w) = spawn_display_worker_on_panel(source, panel.clone());
    let turn = PAGE_DWELL * 3;
    wait_until(|| panel.lock().expect("panel").shows_row_starting_with("GEAR "), REFRESH_WAIT);
    wait_until(|| panel.lock().expect("panel").shows_row_starting_with("FRAMES "), turn);
    assert_eq!(view.snapshot().page, DisplayPage::Bus);
    wait_until(|| panel.lock().expect("panel").shows_row_starting_with("GEAR "), turn);
}
