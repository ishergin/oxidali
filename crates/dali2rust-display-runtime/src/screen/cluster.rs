use super::pages::alarm;
use super::{row_hcl, spread, RowText, ScreenRow, ScreenState, SCREEN_COLS};
use crate::source::{ClusterGlance, ClusterMode};

pub(super) const CLUSTER_ROW: usize = 3;
pub(super) const CLUSTER_TAG_COLS: usize = 9;
const HCL_HEAD_COLS: usize = SCREEN_COLS - CLUSTER_TAG_COLS - 1;

enum ClusterLine {
    Beside(RowText),
    Whole(RowText),
}

pub(super) fn row_hcl_and_cluster(state: &ScreenState) -> ScreenRow {
    match cluster_line(state.sample.cluster) {
        None => row_hcl(state),
        Some(ClusterLine::Whole(text)) => alarm(text),
        Some(ClusterLine::Beside(tag)) => ScreenRow::text(spread(&hcl_head(state), &tag)),
    }
}

pub(crate) fn shows_a_cluster_fault(state: &ScreenState) -> bool {
    matches!(cluster_line(state.sample.cluster), Some(ClusterLine::Whole(_)))
}

fn cluster_line(cluster: ClusterGlance) -> Option<ClusterLine> {
    let node = cluster.node;
    let line = match cluster.mode {
        ClusterMode::Off => return None,
        _ if cluster.pair_split => fault(node, "PAIR SPLIT"),
        ClusterMode::Waiting => fault(node, "NUMBER TAKEN"),
        ClusterMode::BindFailed => fault(node, "PORT NOT BOUND"),
        ClusterMode::NoAddress => tag(format_args!("N{node} NO IP")),
        ClusterMode::Standby | ClusterMode::Listening => tag(format_args!("N{node} LISTEN")),
        ClusterMode::Running if cluster.known == 0 => tag(format_args!("N{node} ALONE")),
        ClusterMode::Running => tag(format_args!("N{node} {}/{}", cluster.heard, cluster.known)),
    };
    Some(line)
}

fn fault(node: u8, what: &str) -> ClusterLine {
    ClusterLine::Whole(RowText::from_fmt(format_args!("NODE {node} {what}")))
}

fn tag(args: core::fmt::Arguments<'_>) -> ClusterLine {
    ClusterLine::Beside(RowText::from_fmt(args))
}

fn hcl_head(state: &ScreenState) -> RowText {
    let mut head = match (state.sample.time_synced, state.hcl_kelvin) {
        (false, _) => RowText::from_fmt(format_args!("HCL NO TIME")),
        (true, Some(kelvin)) => RowText::from_fmt(format_args!("HCL {kelvin}K")),
        (true, None) => RowText::from_fmt(format_args!("HCL IDLE")),
    };
    head.truncate(HCL_HEAD_COLS);
    head
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::DisplayPage;

    use super::*;
    use crate::screen::{render, COLS_WITH_SPARK};
    use crate::source::DisplaySample;

    const NODE: u8 = 3;
    const LAST_NODE: u8 = 7;
    const WIDEST_KELVIN: u16 = u16::MAX;
    const ACTIVITY_HCL_ROW: usize = 5;
    const MODES: [ClusterMode; 7] = [
        ClusterMode::Off,
        ClusterMode::NoAddress,
        ClusterMode::BindFailed,
        ClusterMode::Standby,
        ClusterMode::Listening,
        ClusterMode::Waiting,
        ClusterMode::Running,
    ];

    fn on(mode: ClusterMode) -> ClusterGlance {
        ClusterGlance { mode, node: NODE, ..ClusterGlance::default() }
    }

    fn running(heard: u8, known: u8) -> ClusterGlance {
        ClusterGlance { heard, known, ..on(ClusterMode::Running) }
    }

    fn state_with(cluster: ClusterGlance) -> ScreenState {
        ScreenState {
            ip: Some([10, 0, 0, 2]),
            sample: DisplaySample {
                adapter_enabled: true,
                time_synced: true,
                hcl_overrides_active: 2,
                cluster,
                ..DisplaySample::default()
            },
            hcl_kelvin: Some(6500),
            ..ScreenState::default()
        }
    }

    fn row(cluster: ClusterGlance) -> ScreenRow {
        render(&state_with(cluster))[CLUSTER_ROW]
    }

    #[test]
    fn a_running_node_shows_its_number_and_the_others_it_hears_beside_hcl() {
        let all = row(running(3, 3));
        assert_eq!(all.text, "HCL 6500K      N3 3/3");
        let some = row(running(2, 3));
        assert_eq!(some.text, "HCL 6500K      N3 2/3");
        assert!(!all.invert && !some.invert, "a silent neighbour is not this node's fault");
        assert_eq!(row(running(0, 0)).text, "HCL 6500K    N3 ALONE");
    }

    #[test]
    fn a_node_that_is_not_yet_running_says_why_beside_hcl() {
        for (mode, text) in [
            (ClusterMode::Listening, "HCL 6500K   N3 LISTEN"),
            (ClusterMode::Standby, "HCL 6500K   N3 LISTEN"),
            (ClusterMode::NoAddress, "HCL 6500K    N3 NO IP"),
        ] {
            let shown = row(on(mode));
            assert_eq!(shown.text, text, "{mode:?}");
            assert!(!shown.invert, "{mode:?} is no fault");
        }
    }

    #[test]
    fn a_fault_takes_the_whole_row_inverted() {
        for (cluster, text) in [
            (on(ClusterMode::Waiting), "NODE 3 NUMBER TAKEN"),
            (on(ClusterMode::BindFailed), "NODE 3 PORT NOT BOUND"),
            (ClusterGlance { pair_split: true, ..running(1, 1) }, "NODE 3 PAIR SPLIT"),
        ] {
            let shown = row(cluster);
            assert_eq!(shown.text, text, "{cluster:?}");
            assert!(shown.invert, "{cluster:?}");
            assert!(shows_a_cluster_fault(&state_with(cluster)), "{cluster:?}");
        }
    }

    #[test]
    fn a_split_pair_outranks_every_mode_of_a_cluster_that_is_on() {
        for mode in MODES.into_iter().filter(|mode| *mode != ClusterMode::Off) {
            let split = ClusterGlance { pair_split: true, ..on(mode) };
            assert_eq!(row(split).text, "NODE 3 PAIR SPLIT", "{mode:?}");
        }
    }

    #[test]
    fn only_a_fault_counts_as_one() {
        for mode in [ClusterMode::Off, ClusterMode::NoAddress, ClusterMode::Standby, ClusterMode::Listening] {
            assert!(!shows_a_cluster_fault(&state_with(on(mode))), "{mode:?}");
        }
        assert!(!shows_a_cluster_fault(&state_with(running(0, 7))));
    }

    #[test]
    fn a_cluster_that_is_off_leaves_every_page_as_it_was() {
        let off = state_with(ClusterGlance::default());
        let stale = ClusterGlance { node: LAST_NODE, heard: 5, known: 7, pair_split: true, ..ClusterGlance::default() };
        let still_off = ScreenState { sample: DisplaySample { cluster: stale, ..off.sample }, ..off };
        for page in DisplayPage::ALL {
            let before = render(&ScreenState { page, ..off });
            assert_eq!(before, render(&ScreenState { page, ..still_off }), "{page:?}");
        }
        assert_eq!(render(&off)[CLUSTER_ROW].text, "HCL  6500K      OVR 2");
        assert!(!shows_a_cluster_fault(&still_off));
    }

    #[test]
    fn the_activity_page_keeps_the_override_count_the_summary_gives_up() {
        let state = ScreenState { page: DisplayPage::Activity, ..state_with(running(1, 1)) };
        assert_eq!(render(&state)[ACTIVITY_HCL_ROW].text, "HCL  6500K      OVR 2");
    }

    #[test]
    fn the_node_tag_stays_whole_beside_every_hcl_head() {
        let heads = [(true, Some(WIDEST_KELVIN), "HCL 65535K"), (true, None, "HCL IDLE"), (false, None, "HCL NO TIME")];
        for (synced, kelvin, head) in heads {
            for cluster in [running(LAST_NODE, LAST_NODE), running(0, 0), on(ClusterMode::Listening), on(ClusterMode::NoAddress)] {
                let cluster = ClusterGlance { node: LAST_NODE, ..cluster };
                let mut state = state_with(cluster);
                state.sample.time_synced = synced;
                state.hcl_kelvin = kelvin;
                let shown = render(&state)[CLUSTER_ROW].text;
                let Some(ClusterLine::Beside(tag)) = cluster_line(cluster) else { panic!("{cluster:?}") };
                assert!(tag.len() <= CLUSTER_TAG_COLS, "{tag:?}");
                assert!(shown.ends_with(tag.as_str()) && shown.len() <= SCREEN_COLS, "{shown:?}");
                assert!(shown.starts_with(&format!("{head} ")), "the head stays whole too: {shown:?}");
            }
        }
    }

    #[test]
    fn no_cluster_form_pushes_a_row_past_its_budget_on_any_page() {
        for mode in MODES {
            for pair_split in [false, true] {
                let cluster = ClusterGlance { mode, node: LAST_NODE, heard: LAST_NODE, known: LAST_NODE, pair_split };
                let mut state = state_with(cluster);
                state.sample.time_synced = false;
                for page in DisplayPage::ALL {
                    for (i, shown) in render(&ScreenState { page, ..state }).iter().enumerate() {
                        let budget = if shown.spark { COLS_WITH_SPARK } else { SCREEN_COLS };
                        assert!(shown.text.len() <= budget, "{mode:?} {page:?} row {i}: {:?}", shown.text);
                    }
                }
            }
        }
    }
}
