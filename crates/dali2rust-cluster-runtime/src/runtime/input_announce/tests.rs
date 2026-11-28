use dali2rust_contracts::msg::fixed_text_64;
use dali2rust_domain::registry::{InputDeviceKeys, InputDeviceView, InputInstanceView, CLUSTER_INPUT_DEVICES};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;

use super::*;
use crate::frame::{cluster_tag, decode, Frame, InputInfoElement};
use crate::presence::PresenceTiming;

const TIMING: PresenceTiming = PresenceTiming::every_ms(160);
const WATCH_MS: u32 = 2_000;
const INSTANCES: u8 = 4;
const FULL_PANEL: u8 = 32;
const PRESSED_MS: u32 = 5;

struct Panels {
    devices: u8,
    instances: u8,
    name: &'static str,
    value: Option<u16>,
}

impl ClusterInputReadPort for Panels {
    fn input_device_keys(&self) -> InputDeviceKeys {
        let mut keys = InputDeviceKeys::default();
        (0..self.devices).for_each(|short| keys.insert(short / 64, short % 64));
        keys
    }

    fn input_device_view(&self, adapter_id: u8, short_address: u8) -> Option<InputDeviceView> {
        let name = fixed_text_64(self.name);
        let instance_count = self.instances;
        Some(InputDeviceView { adapter_id, short_address, name, ha_expose: true, present: true, instance_count })
    }

    fn input_instance_view(&self, _adapter_id: u8, _short_address: u8, index: usize) -> Option<InputInstanceView> {
        let instance_number = u8::try_from(index).ok().filter(|number| *number < self.instances)?;
        Some(InputInstanceView {
            instance_number,
            instance_type: Some(1),
            groups: [Some(2), None, None],
            last_event_info: self.value.map(|_| 1),
            input_value: self.value,
            last_event_mono_ms: self.value.map(|_| PRESSED_MS),
        })
    }

    fn input_instance_groups(&self, _adapter_id: u8, _short_address: u8, _instance_number: u8) -> [Option<u8>; 3] {
        [Some(2), None, None]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    Owner,
    Device(u8),
    Instance(u8, u8),
}

struct Rig {
    announcer: InputAnnouncer,
    panels: Panels,
    identity: OwnIdentity,
    buffer: DatagramBuffer,
    pages: Vec<(u32, Vec<Seen>, bool)>,
}

impl Rig {
    fn new(devices: u8, instances: u8) -> Self {
        Self {
            announcer: InputAnnouncer::new(),
            panels: Panels { devices, instances, name: "hall", value: None },
            identity: OwnIdentity::new("hall-controller", "homeassistant", true),
            buffer: [0; MAX_CLUSTER_DATAGRAM_BYTES],
            pages: Vec::new(),
        }
    }

    fn turn(&mut self, now_ms: u32, running: bool, hello: bool) {
        let turn = Turn { running, node: 1, now_ms, hello, split: false, timing: TIMING };
        self.announcer.plan(turn, OwnInputReads { identity: &self.identity, port: &self.panels });
        if self.announcer.page_due(now_ms) {
            let header = Header { cluster_tag: cluster_tag("hall"), sender_mac: [2; 6], boot_epoch: 1, node_no: 1, seq: 0 };
            let len = self.announcer.write_page(&mut self.buffer, &header, now_ms).expect("a page is written");
            self.pages.push(page_seen(now_ms, &self.buffer[..len]));
        }
    }

    fn run(&mut self, from_ms: u32, to_ms: u32) {
        (from_ms..to_ms).for_each(|now_ms| self.turn(now_ms, true, false));
    }

    fn sets(&self) -> usize {
        self.pages.iter().filter(|(_, _, last)| *last).count()
    }
}

fn page_seen(now_ms: u32, bytes: &[u8]) -> (u32, Vec<Seen>, bool) {
    let Frame::InputInfo(page) = decode(bytes).expect("decodes").frame else { panic!("an input info page") };
    let seen = page.elements().map(|element| match element {
        InputInfoElement::Owner(_) => Seen::Owner,
        InputInfoElement::Device(device) => Seen::Device(device.device),
        InputInfoElement::Instance(instance) => Seen::Instance(instance.device, instance.instance),
    });
    (now_ms, seen.collect(), page.meta().last)
}

#[test]
fn a_set_pages_the_owner_then_each_device_followed_by_its_instances() {
    let mut rig = Rig::new(2, INSTANCES);
    rig.run(0, TIMING.info_coalesce_ms() + 1);
    let seen: Vec<Seen> = rig.pages.iter().flat_map(|(_, seen, _)| seen.clone()).collect();
    let mut expected = vec![Seen::Owner];
    for device in 0..2 {
        expected.push(Seen::Device(device));
        expected.extend((0..INSTANCES).map(|instance| Seen::Instance(device, instance)));
    }
    assert_eq!(seen, expected);
    assert_eq!(rig.sets(), 1);
}

#[test]
fn a_full_panel_set_goes_out_over_pages_a_page_gap_apart_and_closes_once() {
    let devices = u8::try_from(CLUSTER_INPUT_DEVICES).expect("a device count");
    let mut rig = Rig::new(devices + 1, FULL_PANEL);
    rig.run(0, TIMING.info_coalesce_ms() + 1_000);
    let first_set: Vec<_> = rig.pages.iter().take_while(|(_, _, last)| !*last).chain(rig.pages.iter().find(|(_, _, last)| *last)).collect();
    assert!(first_set.len() > 1, "a full panel takes several pages");
    assert!(first_set.windows(2).all(|pair| pair[1].0 - pair[0].0 == PAGE_GAP_MS));
    let listed = first_set.iter().flat_map(|(_, seen, _)| seen.iter()).filter(|seen| matches!(seen, Seen::Device(_))).count();
    assert_eq!(listed, CLUSTER_INPUT_DEVICES, "the node lists at most its device slots");
}

#[test]
fn a_new_value_alone_starts_no_set_and_a_rename_does_after_the_coalescing_window() {
    let mut rig = Rig::new(1, INSTANCES);
    let coalesce = TIMING.info_coalesce_ms();
    rig.run(0, coalesce + 1);
    assert_eq!(rig.sets(), 1);
    rig.panels.value = Some(7);
    rig.announcer.note_changed();
    rig.run(coalesce + 1, 3 * coalesce);
    assert_eq!(rig.sets(), 1, "a value rides the event and the refresh, not a new set");
    rig.panels.name = "porch";
    rig.announcer.note_changed();
    let renamed_ms = 3 * coalesce;
    rig.run(renamed_ms, renamed_ms + coalesce + 1);
    assert_eq!(rig.pages.last().map(|(at_ms, _, last)| (*at_ms, *last)), Some((renamed_ms + coalesce, true)));
}

#[test]
fn the_refresh_resends_the_set_and_a_hello_gets_its_own_answer() {
    let mut rig = Rig::new(1, INSTANCES);
    let watch_ms = 2 * TIMING.info_refresh_ms();
    rig.run(0, watch_ms);
    assert_eq!(rig.sets(), 3, "the first set and two refreshes");
    rig.turn(watch_ms, true, true);
    let answer_ms = watch_ms + hello_delay_ms(Turn { running: true, node: 1, now_ms: watch_ms, hello: true, split: false, timing: TIMING });
    rig.run(watch_ms + 1, answer_ms + 1);
    assert_eq!(rig.pages.last().map(|(at_ms, _, _)| *at_ms), Some(answer_ms));
    assert_eq!(rig.sets(), 4);
}

#[test]
fn a_node_that_is_not_running_pages_nothing_and_forgets_what_it_announced() {
    let mut rig = Rig::new(1, INSTANCES);
    rig.run(0, TIMING.info_coalesce_ms() + 1);
    assert!(rig.announcer.announced().is_some());
    let stopped_at = rig.pages.len();
    for now_ms in TIMING.info_coalesce_ms() + 1..WATCH_MS {
        rig.turn(now_ms, false, true);
    }
    assert_eq!(rig.pages.len(), stopped_at);
    assert_eq!((rig.announcer.announced(), rig.announcer.deadline()), (None, None));
}
