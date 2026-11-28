use std::sync::Mutex;

use dali2rust_contracts::msg::{fixed_text_32, fixed_text_64, ClusterRecordKind, ColorMode, DeviceType, PowerState};
use dali2rust_domain::registry::{
    CapabilityFlagsView, ClusterGroupLitView, ClusterHaIdentityView, ClusterLampInfoView, ClusterLampStateView,
    ClusterPartKeys, ClusterPartView,
};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;

use super::*;
use crate::frame::{cluster_tag, decode, Frame, LampInfoElement, StateElement};
use crate::lamps::OLDEST_OBSERVATION_AGE_MS;
use crate::presence::{PresenceTiming, HALF_CLOCK_MS};

const TIMING: PresenceTiming = PresenceTiming::every_ms(160);
const ALL_LAMPS: u64 = u64::MAX;
const ONE_LAMP: u64 = 1;
const LONG_NAME: &str = "A lamp whose name fills most of the sixty-four bytes it may hold";
const OBSERVED_AT_MS: u32 = 0;
const SETTLED_MS: u32 = TIMING.info_coalesce_ms() * 2 + PAGE_GAP_MS * NODE_COUNT as u32;

struct Lamps {
    ids: u64,
    level: Mutex<Option<u8>>,
    name: Mutex<&'static str>,
}

impl ClusterLampReadPort for Lamps {
    fn cluster_ha_identity(&self) -> ClusterHaIdentityView {
        ClusterHaIdentityView { controller_id: fixed_text_32("ctl"), discovery_prefix: fixed_text_32("homeassistant") }
    }

    fn cluster_lamp_ids(&self, _adapter: u8) -> u64 {
        self.ids
    }

    fn cluster_lamp_state(&self, _adapter: u8, _lamp: u8) -> Option<ClusterLampStateView> {
        let level = *self.level.lock().expect("level");
        Some(ClusterLampStateView {
            power: if level.is_some() { PowerState::On } else { PowerState::Unknown },
            level,
            color_mode: ColorMode::Brightness,
            kelvin: None,
            xy: None,
            rgb: None,
            waf: None,
            observed_at_ms: Some(OBSERVED_AT_MS),
            value_source: None,
            absent: false,
        })
    }

    fn cluster_lamp_info(&self, _adapter: u8, _lamp: u8) -> Option<ClusterLampInfoView> {
        Some(ClusterLampInfoView {
            name: fixed_text_64(&self.name.lock().expect("name")),
            device_type: DeviceType::Dt6Led,
            device_type_overridden: false,
            device_types: None,
            color_mode: ColorMode::Brightness,
            color_mode_overridden: false,
            capabilities: CapabilityFlagsView::default(),
            kelvin_range: None,
            bound: true,
            ha_entity: true,
        })
    }

    fn cluster_group_lit(&self, _adapter: u8, _group: u8) -> ClusterGroupLitView {
        ClusterGroupLitView::default()
    }
}

struct NoParts;

impl ClusterPartReadPort for NoParts {
    fn cluster_part(&self, _kind: ClusterRecordKind, _adapter: u8, _local: u8) -> Option<ClusterPartView> {
        None
    }

    fn cluster_part_keys(&self, _kind: ClusterRecordKind) -> ClusterPartKeys {
        ClusterPartKeys::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sent {
    at_ms: u32,
    subject: Subject,
    closes_set: bool,
    first_is_owner: bool,
}

struct Rig {
    node: u8,
    announcer: LampAnnouncer,
    lamps: Lamps,
    buffer: DatagramBuffer,
    sent: Vec<Sent>,
}

impl Rig {
    fn new(node: u8, ids: u64) -> Self {
        let lamps = Lamps { ids, level: Mutex::new(Some(100)), name: Mutex::new("Hall") };
        let buffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
        Self { node, announcer: LampAnnouncer::new(), lamps, buffer, sent: Vec::new() }
    }

    fn turn(&mut self, now_ms: u32, running: bool, hello: bool) {
        let turn = Turn { running, node: self.node, now_ms, hello, split: false, timing: TIMING };
        self.announcer.plan(turn, OwnLamps { lamps: &self.lamps, parts: &NoParts });
        if !self.announcer.page_due(now_ms) {
            return;
        }
        let tag = cluster_tag("hall");
        let header = Header { cluster_tag: tag, sender_mac: [2; 6], boot_epoch: 1, node_no: self.node, seq: 0 };
        let len = self.announcer.write_page(&mut self.buffer, &header, now_ms).expect("a page is written");
        let sent = match decode(&self.buffer[..len]).expect("decodes").frame {
            Frame::LampState(page) => {
                assert!(page.elements().all(|element| matches!(element, StateElement::Lamp(_))));
                Sent { at_ms: now_ms, subject: Subject::State, closes_set: page.meta().last, first_is_owner: false }
            }
            Frame::LampInfo(page) => {
                let first_is_owner = matches!(page.elements().next(), Some(LampInfoElement::Owner(_)));
                Sent { at_ms: now_ms, subject: Subject::Info, closes_set: page.meta().last, first_is_owner }
            }
            other => panic!("not a lamp page: {other:?}"),
        };
        self.sent.push(sent);
    }

    fn run(&mut self, from_ms: u32, to_ms: u32) {
        for now_ms in from_ms..to_ms {
            self.turn(now_ms, true, false);
        }
    }

    fn sets(&self, subject: Subject) -> Vec<u32> {
        let mut opens = true;
        let mut starts = Vec::new();
        for sent in self.sent.iter().filter(|sent| sent.subject == subject) {
            if opens {
                starts.push(sent.at_ms);
            }
            opens = sent.closes_set;
        }
        starts
    }

    fn settled(node: u8) -> Self {
        let mut rig = Self::new(node, ONE_LAMP);
        rig.run(0, SETTLED_MS);
        assert_eq!(rig.sets(Subject::State).len(), 1, "the first state set went out");
        rig.sent.clear();
        rig
    }
}

fn first_refresh_ms(node: u8) -> u32 {
    let slot_ms = TIMING.info_refresh_ms() / u32::from(NODE_COUNT);
    slot_ms * u32::from(node) + slot_ms / PHASE_HALVES
}

#[test]
fn nothing_is_announced_while_the_node_is_not_running() {
    let mut rig = Rig::new(0, ONE_LAMP);
    rig.announcer.note_moved(Subject::State, 0);
    for now_ms in 0..TIMING.info_refresh_ms() * 2 {
        rig.turn(now_ms, false, true);
    }
    assert!(rig.sent.is_empty());
}

#[test]
fn moves_within_one_coalescing_window_go_out_as_one_state_set() {
    let mut rig = Rig::settled(0);
    let from_ms = SETTLED_MS;
    for (step, level) in [10_u8, 20, 30].into_iter().enumerate() {
        let at_ms = from_ms + u32::try_from(step).expect("small") * 5;
        *rig.lamps.level.lock().expect("level") = Some(level);
        rig.announcer.note_moved(Subject::State, at_ms);
        rig.run(at_ms, at_ms + 5);
    }
    rig.run(from_ms + 15, from_ms + TIMING.info_coalesce_ms() * 2);
    assert_eq!(rig.sets(Subject::State), vec![from_ms + TIMING.info_coalesce_ms()]);
}

#[test]
fn a_move_that_changed_nothing_but_the_age_sends_nothing() {
    let mut rig = Rig::settled(0);
    let at_ms = SETTLED_MS;
    rig.announcer.note_moved(Subject::State, at_ms);
    rig.run(at_ms, at_ms + TIMING.info_coalesce_ms() * 2);
    assert!(rig.sent.is_empty(), "{:?}", rig.sent);
}

#[test]
fn the_refresh_resends_both_sets_unchanged_at_the_nodes_phase() {
    let node = 3;
    let mut rig = Rig::new(node, ONE_LAMP);
    let at_ms = first_refresh_ms(node) + TIMING.info_refresh_ms();
    let started_ms = at_ms - TIMING.info_refresh_ms() / 2 - TIMING.info_coalesce_ms() * 4;
    rig.run(started_ms, at_ms + TIMING.info_coalesce_ms());
    let sets = rig.sets(Subject::State);
    assert_eq!((sets.len(), sets.last()), (2, Some(&at_ms)), "the first set, then the refresh: {sets:?}");
    assert_eq!(rig.sets(Subject::Info).len(), 2);
}

#[test]
fn a_refresh_within_half_a_period_of_the_last_set_is_skipped() {
    let node = 3;
    let mut rig = Rig::settled(node);
    let at_ms = first_refresh_ms(node);
    assert!(at_ms - SETTLED_MS < TIMING.info_refresh_ms() / 2, "the settling set is recent at the first refresh");
    rig.run(SETTLED_MS, at_ms + TIMING.info_coalesce_ms());
    assert!(rig.sent.is_empty(), "the receivers are fresh, so nothing is resent: {:?}", rig.sent);
}

#[test]
fn any_number_of_hellos_get_one_answer_of_each_set() {
    let node = 2;
    let mut rig = Rig::settled(node);
    let from_ms = SETTLED_MS;
    for now_ms in from_ms..from_ms + TIMING.info_coalesce_ms() * 6 {
        rig.turn(now_ms, true, now_ms < from_ms + 4);
    }
    assert_eq!(rig.sets(Subject::State), vec![from_ms + hello_delay_ms(Turn {
        running: true,
        node,
        now_ms: 0,
        hello: true,
        split: false,
        timing: TIMING
    })]);
    assert_eq!(rig.sets(Subject::Info).len(), 1);
}

#[test]
fn a_rename_resends_the_info_set_and_it_opens_with_the_owner() {
    let mut rig = Rig::settled(0);
    let at_ms = SETTLED_MS;
    *rig.lamps.name.lock().expect("name") = "Kitchen";
    rig.announcer.note_moved(Subject::Info, at_ms);
    rig.run(at_ms, at_ms + TIMING.info_coalesce_ms() * 2);
    assert_eq!(rig.sets(Subject::Info), vec![at_ms + TIMING.info_coalesce_ms()]);
    assert!(rig.sent.first().is_some_and(|sent| sent.first_is_owner));
}

#[test]
fn a_paged_set_goes_out_a_page_gap_apart_and_closes_once() {
    let mut rig = Rig::new(0, ALL_LAMPS);
    *rig.lamps.name.lock().expect("name") = LONG_NAME;
    rig.run(0, SETTLED_MS * 2);
    let info: Vec<u32> = rig.sent.iter().filter(|sent| sent.subject == Subject::Info).map(|sent| sent.at_ms).collect();
    assert!(info.len() > 1, "64 long names take more than one page: {info:?}");
    assert!(info.windows(2).all(|pair| pair[1] - pair[0] == PAGE_GAP_MS), "{info:?}");
    assert_eq!(rig.sent.iter().filter(|sent| sent.subject == Subject::Info && sent.closes_set).count(), 1);
}

#[test]
fn a_lamp_observed_more_than_half_a_clock_ago_is_announced_at_its_oldest_age_never_as_fresh() {
    let lamps = Lamps { ids: ONE_LAMP, level: Mutex::new(Some(100)), name: Mutex::new("Hall") };
    let mut set = placed::<StateSet>();
    for (since_ms, age_ms) in [(5, 5), (HALF_CLOCK_MS + 5, OLDEST_OBSERVATION_AGE_MS)] {
        fill_state(&mut set, (&lamps, &NoParts), ANNOUNCED_ADAPTER, OBSERVED_AT_MS.wrapping_add(since_ms));
        let announced = set.lamp(0).and_then(|lamp| lamp.age_ms);
        assert_eq!(announced, Some(age_ms), "a lamp observed {since_ms} ms ago");
    }
}
