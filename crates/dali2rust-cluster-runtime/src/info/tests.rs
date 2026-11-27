use dali2rust_contracts::msg::ClusterRecordKind::{self, Channel, Scene};
use dali2rust_domain::registry::{
    ClusterPartKeys, ClusterPartReadPort, ClusterPartView, ClusterRecordNumbers,
    ClusterRecordReadPort, ClusterRecordView,
};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;

use super::*;
use crate::frame::{
    decode, BirthMark, ChannelPart, DatagramBuffer, Frame, GroupMap, Header, InfoElement, InfoPageWriter,
    LocalName, PageCursor, PartFlags, Record, RecordRef, ScenePart,
};

const NODE: u8 = 2;
const MAC: [u8; 6] = [2, 0, 0, 0, 0, 2];
const EPOCH: u32 = 0x00e0_0001;
const NOW_MS: u32 = 5_000;
const OWN_NUMBER: u8 = 34;
const FOREIGN_NUMBER: u8 = 3;
const MARK: BirthMark = BirthMark { generation: 1, nonce: 0xfeed };

fn record(number: u8, name: &str, revision: u32) -> InfoElement<'_> {
    InfoElement::ChannelRecord(Record { id: RecordRef { number, mark: MARK }, name, revision })
}

fn channel_part(group: u8, number: u8) -> ChannelPart {
    ChannelPart {
        adapter: 0,
        channel: RecordRef { number, mark: MARK },
        group,
        taken: false,
        flags: PartFlags { send: true, accept: false },
        revision: 4,
        desired_lamps: 0,
        written_lamps: 0,
    }
}

fn page_bytes(set: u32, page: u8, last: bool, elements: &[InfoElement<'_>]) -> Vec<u8> {
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let header = Header { cluster_tag: 1, sender_mac: MAC, boot_epoch: EPOCH, node_no: NODE, seq: 0 };
    let mut writer = InfoPageWriter::start(&mut buffer, &header, set, page).expect("start");
    for element in elements {
        writer.push(element).expect("fits");
    }
    let len = writer.finish(last);
    buffer[..len].to_vec()
}

fn feed(cache: &mut InfoCache, bytes: &[u8], epoch: u32) -> PageTally {
    let Frame::ChannelSceneInfo(page) = decode(bytes).expect("decode").frame else { panic!("not a page") };
    cache.take_page(Arrival { node: NODE, mac: MAC, epoch, now_ms: NOW_MS }, &page)
}

fn names(cache: &InfoCache) -> Vec<String> {
    let Some(set) = cache.shown(NODE) else { return Vec::new() };
    set.record_numbers(Channel).iter().filter_map(|n| set.record_view(Channel, n)).map(|view| view.name).collect()
}

#[test]
fn a_whole_set_in_one_page_is_shown_with_its_records_and_parts() {
    let mut cache = placed_cache();
    let tally = feed(&mut cache, &page_bytes(1, 0, true, &[record(OWN_NUMBER, "Hall", 2)]), EPOCH);
    assert!(tally.taken && tally.changed && !tally.stale && !tally.abandoned);
    assert_eq!(names(&cache), ["Hall"]);
    let view = cache.shown(NODE).and_then(|set| set.record_view(Channel, OWN_NUMBER)).expect("held");
    assert_eq!((view.home_node, view.mark, view.revision), (NODE, MARK, 2));
}

#[test]
fn a_set_is_shown_only_once_its_last_page_arrived() {
    let mut cache = placed_cache();
    assert!(!feed(&mut cache, &page_bytes(1, 0, false, &[record(OWN_NUMBER, "Hall", 1)]), EPOCH).taken);
    assert!(cache.shown(NODE).is_none());
    assert!(feed(&mut cache, &page_bytes(1, 1, true, &[record(OWN_NUMBER + 1, "Porch", 1)]), EPOCH).taken);
    assert_eq!(names(&cache), ["Hall", "Porch"]);
}

#[test]
fn a_lost_page_abandons_the_set_and_keeps_the_previous_whole_one() {
    let mut cache = placed_cache();
    feed(&mut cache, &page_bytes(1, 0, true, &[record(OWN_NUMBER, "Hall", 1)]), EPOCH);
    feed(&mut cache, &page_bytes(2, 0, false, &[record(OWN_NUMBER, "Renamed", 2)]), EPOCH);
    let tally = feed(&mut cache, &page_bytes(2, 2, true, &[]), EPOCH);
    assert!(tally.stale && tally.abandoned && !tally.taken);
    assert_eq!(names(&cache), ["Hall"]);
    assert!(feed(&mut cache, &page_bytes(2, 1, true, &[]), EPOCH).stale);
    assert_eq!(names(&cache), ["Hall"]);
}

#[test]
fn a_newer_set_replaces_the_older_and_an_older_one_is_stale() {
    let mut cache = placed_cache();
    feed(&mut cache, &page_bytes(5, 0, true, &[record(OWN_NUMBER, "Hall", 1)]), EPOCH);
    assert!(feed(&mut cache, &page_bytes(6, 0, true, &[record(OWN_NUMBER, "Lobby", 2)]), EPOCH).changed);
    assert_eq!(names(&cache), ["Lobby"]);
    assert!(feed(&mut cache, &page_bytes(4, 0, true, &[record(OWN_NUMBER, "Old", 1)]), EPOCH).stale);
    assert!(feed(&mut cache, &page_bytes(6, 0, true, &[record(OWN_NUMBER, "Same", 1)]), EPOCH).stale);
    assert_eq!(names(&cache), ["Lobby"]);
}

#[test]
fn a_newer_set_abandons_a_set_still_arriving() {
    let mut cache = placed_cache();
    feed(&mut cache, &page_bytes(1, 0, false, &[record(OWN_NUMBER, "Hall", 1)]), EPOCH);
    let tally = feed(&mut cache, &page_bytes(2, 0, true, &[record(OWN_NUMBER, "Lobby", 2)]), EPOCH);
    assert!(tally.abandoned && tally.taken);
    assert_eq!(names(&cache), ["Lobby"]);
}

#[test]
fn a_restarted_board_starts_its_sets_again_and_still_replaces_the_held_one() {
    let mut cache = placed_cache();
    feed(&mut cache, &page_bytes(90, 0, true, &[record(OWN_NUMBER, "Hall", 1)]), EPOCH);
    assert!(feed(&mut cache, &page_bytes(0, 0, true, &[record(OWN_NUMBER, "Fresh", 1)]), EPOCH + 1).taken);
    assert_eq!(names(&cache), ["Fresh"]);
}

#[test]
fn an_empty_set_clears_the_node_s_info_but_keeps_it_held() {
    let mut cache = placed_cache();
    feed(&mut cache, &page_bytes(1, 0, true, &[record(OWN_NUMBER, "Hall", 1)]), EPOCH);
    assert!(feed(&mut cache, &page_bytes(2, 0, true, &[]), EPOCH).changed);
    assert_eq!(names(&cache), Vec::<String>::new());
    assert_eq!(cache.counts(NODE, Channel), Some((0, 0)));
}

#[test]
fn the_same_content_again_is_taken_but_is_not_a_change() {
    let mut cache = placed_cache();
    feed(&mut cache, &page_bytes(1, 0, true, &[record(OWN_NUMBER, "Hall", 1)]), EPOCH);
    let before = cache.changes();
    let tally = feed(&mut cache, &page_bytes(2, 0, true, &[record(OWN_NUMBER, "Hall", 1)]), EPOCH);
    assert!(tally.taken && !tally.changed);
    assert_eq!(cache.changes(), before);
}

#[test]
fn a_record_outside_the_sender_s_block_is_skipped_and_proves_nothing() {
    let mut cache = placed_cache();
    let tally = feed(&mut cache, &page_bytes(1, 0, true, &[record(FOREIGN_NUMBER, "Copy", 9)]), EPOCH);
    assert!(tally.taken);
    assert_eq!(tally.skipped, 1);
    let set = cache.shown(NODE).expect("held");
    assert!(set.record_view(Channel, FOREIGN_NUMBER).is_none());
    assert!(!set.record_numbers(Channel).any());
}

#[test]
fn elements_of_later_slices_are_skipped_without_failing_the_set() {
    let mut cache = placed_cache();
    let name = LocalName { adapter: 0, number: 1, name: "Kitchen" };
    let mut mapped = channel_part(5, OWN_NUMBER);
    mapped.desired_lamps = 0b101;
    let elements = [
        InfoElement::GroupMap(GroupMap { adapter: 0, occupied: 0xffff }),
        InfoElement::GroupName(name),
        InfoElement::SceneName(name),
        InfoElement::ChannelPart(mapped),
        record(OWN_NUMBER, "Hall", 1),
    ];
    let tally = feed(&mut cache, &page_bytes(1, 0, true, &elements), EPOCH);
    assert!(tally.taken);
    assert_eq!(tally.skipped, 4);
    let part = cache.shown(NODE).and_then(|set| set.part_view(Channel, 0, 5)).expect("the part is kept");
    assert_eq!((part.record.number, part.send, part.accept, part.revision), (OWN_NUMBER, true, false, 4));
    assert_eq!(cache.held(NODE, NOW_MS).map(|held| held.skipped), Some(4));
}

#[test]
fn a_scene_part_and_a_scene_record_land_under_the_scene_kind() {
    let mut cache = placed_cache();
    let scene = ScenePart {
        adapter: 1,
        scene: RecordRef { number: 40, mark: MARK },
        node_scene: 7,
        flags: PartFlags { send: false, accept: true },
        revision: 2,
    };
    let scene_record = InfoElement::SceneRecord(Record { id: RecordRef { number: 33, mark: MARK }, name: "Eve", revision: 1 });
    feed(&mut cache, &page_bytes(1, 0, true, &[scene_record, InfoElement::ScenePart(scene)]), EPOCH);
    let set = cache.shown(NODE).expect("held");
    assert_eq!(set.record_view(Scene, 33).map(|view| view.name), Some("Eve".to_owned()));
    assert!(set.record_view(Channel, 33).is_none());
    assert_eq!(set.part_view(Scene, 1, 7).map(|view| (view.record.number, view.accept)), Some((40, true)));
    assert_eq!(set.part_key_set(Scene).iter().collect::<Vec<_>>(), [(1, 7)]);
    assert!(!set.part_key_set(Channel).iter().any(|_| true));
}

#[test]
fn forget_drops_one_node_and_clear_drops_all() {
    let mut cache = placed_cache();
    feed(&mut cache, &page_bytes(1, 0, true, &[]), EPOCH);
    let before = cache.changes();
    cache.forget(NODE);
    assert!(cache.shown(NODE).is_none());
    assert_ne!(cache.changes(), before);
    feed(&mut cache, &page_bytes(2, 0, true, &[]), EPOCH);
    cache.clear();
    assert!(cache.held(NODE, NOW_MS).is_none());
}

#[test]
fn a_page_of_a_set_never_opened_is_stale() {
    let mut cache = placed_cache();
    let tally = feed(&mut cache, &page_bytes(1, 1, true, &[]), EPOCH);
    assert!(tally.stale && !tally.abandoned && !tally.taken);
}

#[test]
fn the_age_of_a_held_set_grows_from_the_moment_it_was_taken() {
    let mut cache = placed_cache();
    feed(&mut cache, &page_bytes(7, 0, true, &[]), EPOCH);
    let held = cache.held(NODE, NOW_MS + 1_500).expect("held");
    assert_eq!((held.set, held.age_ms), (7, 1_500));
}

struct OwnPorts;

impl ClusterRecordReadPort for OwnPorts {
    fn cluster_record(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView> {
        let name = match (kind, number) {
            (ClusterRecordKind::Channel, OWN_NUMBER) => "Hall",
            (ClusterRecordKind::Scene, 33) => "Eve",
            _ => return None,
        };
        Some(ClusterRecordView { kind, number, home_node: NODE, mark: MARK, name: name.to_owned(), revision: 3 })
    }

    fn cluster_record_numbers(&self, kind: ClusterRecordKind) -> ClusterRecordNumbers {
        let live = match kind {
            ClusterRecordKind::Channel => 1 << 2,
            ClusterRecordKind::Scene => 1 << 1,
        };
        ClusterRecordNumbers { first: 32, live }
    }
}

impl ClusterPartReadPort for OwnPorts {
    fn cluster_part(&self, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartView> {
        let record = RecordRef { number: FOREIGN_NUMBER, mark: MARK };
        Some(ClusterPartView { kind, adapter, local, record, send: true, accept: true, revision: 1 })
    }

    fn cluster_part_keys(&self, _kind: ClusterRecordKind) -> ClusterPartKeys {
        let mut keys = ClusterPartKeys::default();
        keys.insert(0, 3);
        keys.insert(7, 15);
        keys
    }
}

#[test]
fn a_node_s_own_info_goes_out_in_pages_and_comes_back_whole() {
    let mut own = placed_set();
    own.fill(NODE, &OwnPorts, &OwnPorts);
    let mut cache = placed_cache();
    let mut cursor = PageCursor::new(1);
    let mut buffer: DatagramBuffer = [0; MAX_CLUSTER_DATAGRAM_BYTES];
    let header = Header { cluster_tag: 1, sender_mac: MAC, boot_epoch: EPOCH, node_no: NODE, seq: 0 };
    while let Some(len) = cursor.write_next(&mut buffer, &header, own.elements()).expect("page") {
        feed(&mut cache, &buffer[..len], EPOCH);
    }
    let shown = cache.shown(NODE).expect("held");
    assert_eq!(*shown, *own);
    assert_eq!(cache.counts(NODE, Channel), Some((1, 2)));
    assert_eq!(cache.counts(NODE, Scene), Some((1, 2)));
}
