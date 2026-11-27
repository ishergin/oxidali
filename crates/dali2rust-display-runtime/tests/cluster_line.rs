mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_display_runtime::{ClusterGlance, ClusterMode, RecordingPanel};
use dali2rust_test_support::wait_until;

use support::{healthy_sample, spawn_display_worker_on_panel, MovingDisplaySource};

const REFRESH_WAIT: Duration = Duration::from_secs(3);

fn shows(panel: &Arc<Mutex<RecordingPanel>>, row_ending: &str) -> bool {
    panel.lock().expect("panel").shows_row_ending_with(row_ending)
}

#[test]
fn a_change_of_the_cluster_reaches_the_panel_and_switching_it_off_restores_the_hcl_row() {
    let source = Arc::new(MovingDisplaySource::new(healthy_sample()));
    let panel = Arc::new(Mutex::new(RecordingPanel::default()));
    let (_host, _publisher, _view, _w) = spawn_display_worker_on_panel(source.clone(), panel.clone());
    wait_until(|| shows(&panel, "OVR 0"), REFRESH_WAIT);
    let node = ClusterGlance { mode: ClusterMode::Running, node: 4, heard: 1, known: 2, pair_split: false };
    source.update(|sample| sample.cluster = node);
    wait_until(|| shows(&panel, "N4 1/2"), REFRESH_WAIT);
    source.update(|sample| sample.cluster.heard = 2);
    wait_until(|| shows(&panel, "N4 2/2"), REFRESH_WAIT);
    source.update(|sample| sample.cluster.mode = ClusterMode::Waiting);
    wait_until(|| panel.lock().expect("panel").shows_row_starting_with("NODE 4 NUMBER TAKEN"), REFRESH_WAIT);
    source.update(|sample| sample.cluster = ClusterGlance::default());
    wait_until(|| shows(&panel, "OVR 0"), REFRESH_WAIT);
}
