use std::time::Duration;

use cucumber::then;
use dali2rust_adapters::RecordingPanel;

use crate::steps::poller_steps::probe_within;
use crate::DaliWorld;

const PANEL_WAIT: Duration = Duration::from_secs(15);
const ROTATION_UPSIDE_DOWN_DEG: u16 = 180;

fn panel_reaches(world: &DaliWorld, holds: impl Fn(&RecordingPanel) -> bool) -> bool {
    let panel = world.display_panel();
    probe_within(PANEL_WAIT, || holds(&panel.lock().expect("display panel")))
}

fn panel_state(world: &DaliWorld) -> String {
    format!("{:?}", world.display_panel().lock().expect("display panel"))
}

// SET-DISP-009, SET-DISP-010
#[then(expr = "the display panel should eventually be configured at I2C address {int} with rotation {int}")]
fn panel_configured(world: &mut DaliWorld, address: u8, rotation_deg: u16) {
    let upside_down = rotation_deg == ROTATION_UPSIDE_DOWN_DEG;
    let reached = panel_reaches(world, |panel| {
        panel
            .setup
            .is_some_and(|setup| setup.i2c_address == address && setup.upside_down == upside_down)
    });
    assert!(reached, "panel never configured at {address} / {rotation_deg}: {}", panel_state(world));
}

// SET-DISP-009, SET-DISP-010, SET-DISP-011
#[then(expr = "the display panel should eventually be powered {word}")]
fn panel_powered(world: &mut DaliWorld, state: String) {
    let on = match state.as_str() {
        "on" => true,
        "off" => false,
        other => panic!("unknown power state {other}"),
    };
    let reached = panel_reaches(world, |panel| panel.configured > 0 && panel.powered == on);
    assert!(reached, "panel never powered {state}: {}", panel_state(world));
}

// SET-DISP-009, SET-DISP-011, SET-DISP-012, SET-DISP-013, SET-DALI-024
#[then(expr = "the display panel should eventually show a row starting with {string}")]
fn panel_shows(world: &mut DaliWorld, prefix: String) {
    let reached = panel_reaches(world, |panel| panel.shows_row_starting_with(&prefix));
    assert!(reached, "panel never showed {prefix:?}: {}", panel_state(world));
}

// SET-DALI-024
#[then(expr = "the display panel should show a row starting with {string} within {int} ms")]
fn panel_shows_within(world: &mut DaliWorld, prefix: String, millis: u64) {
    let panel = world.display_panel();
    let reached = probe_within(Duration::from_millis(millis), || {
        panel.lock().expect("display panel").shows_row_starting_with(&prefix)
    });
    assert!(reached, "panel did not show {prefix:?} in {millis} ms: {}", panel_state(world));
}

// SET-DALI-023
#[then(expr = "the display panel should eventually show a row ending with {string}")]
fn panel_shows_tail(world: &mut DaliWorld, suffix: String) {
    let reached = panel_reaches(world, |panel| panel.shows_row_ending_with(&suffix));
    assert!(reached, "panel never showed a row ending {suffix:?}: {}", panel_state(world));
}

// SET-DISP-013
#[then(expr = "the display panel should keep showing a row starting with {string} for {int} seconds")]
fn panel_keeps_showing(world: &mut DaliWorld, prefix: String, seconds: u64) {
    let panel = world.display_panel();
    let dropped = probe_within(Duration::from_secs(seconds), || {
        !panel.lock().expect("display panel").shows_row_starting_with(&prefix)
    });
    assert!(!dropped, "panel stopped showing {prefix:?}: {}", panel_state(world));
}
