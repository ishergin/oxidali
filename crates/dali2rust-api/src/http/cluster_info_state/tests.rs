use std::collections::BTreeMap;

use super::*;
use crate::http::cluster_records_state::ClusterBirthMarkDto;

const KIND: ClusterRecordKind = ClusterRecordKind::Channel;
const OWN: u8 = 0;
const NEIGHBOUR: u8 = 1;
const FAR: u8 = 2;

fn mark(generation: u32) -> ClusterBirthMarkDto {
    ClusterBirthMarkDto { generation, nonce: 7 }
}

fn record(number: u8, generation: u32, name: &str, source: ClusterInfoSource) -> ClusterRecordDto {
    let home_node = home_node(number);
    ClusterRecordDto { number, home_node, birth_mark: mark(generation), name: name.into(), revision: 1, source }
}

fn part(adapter: u8, group: u8, number: u8, generation: u32) -> ClusterPartDto {
    let record = ClusterPartRecordDto { number, birth_mark: mark(generation) };
    ClusterPartDto { adapter, group: Some(group), scene: None, record, send: true, accept: true, revision: 1 }
}

#[derive(Default)]
struct Cluster {
    sources: BTreeMap<u8, (ClusterInfoSource, u8)>,
    records: BTreeMap<(u8, u8), ClusterRecordDto>,
    parts: BTreeMap<(u8, u8, u8), ClusterPartDto>,
    copies: Vec<ClusterRecordDto>,
}

impl ClusterInfoHttpState for Cluster {
    fn node_info(&self, node: u8) -> Option<ClusterInfoNodeDto> {
        let (source, _) = *self.sources.get(&node)?;
        let (channels, scenes, channel_parts, scene_parts) = (0, 0, 0, 0);
        Some(ClusterInfoNodeDto { node, source, set: None, age_ms: None, channels, scenes, channel_parts, scene_parts, skipped: 0 })
    }

    fn record(&self, node: u8, _kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordDto> {
        self.records.get(&(node, number)).cloned()
    }

    fn record_numbers(&self, node: u8, _kind: ClusterRecordKind) -> ClusterRecordNumbers {
        let first = node * CLUSTER_NUMBERS_PER_NODE;
        let live = self.records.keys().filter(|(at, _)| *at == node).fold(0, |live, (_, number)| live | 1 << (number - first));
        ClusterRecordNumbers { first, live }
    }

    fn part(&self, node: u8, _kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartDto> {
        self.parts.get(&(node, adapter, local)).copied()
    }

    fn part_keys(&self, node: u8, _kind: ClusterRecordKind) -> ClusterPartKeys {
        let mut keys = ClusterPartKeys::default();
        self.parts.keys().filter(|(at, ..)| *at == node).for_each(|(_, adapter, local)| keys.insert(*adapter, *local));
        keys
    }

    fn copy_at(&self, _kind: ClusterRecordKind, index: usize) -> Option<ClusterRecordDto> {
        self.copies.get(index).cloned()
    }

    fn link(&self, _kind: ClusterRecordKind, _record: ClusterRecordRef) -> ClusterPartLink {
        ClusterPartLink::Unknown
    }

    fn wire(&self, node: u8) -> Option<u8> {
        self.sources.get(&node).map(|(_, wire)| *wire)
    }

    fn record_write_refusal(&self) -> Option<&'static str> {
        None
    }
}

fn names(cluster: &Cluster) -> Vec<(u8, String, ClusterInfoSource)> {
    ClusterView::read(cluster).records(KIND).map(|record| (record.number, record.name, record.source)).collect()
}

#[test]
fn a_record_known_only_by_its_copy_is_listed_once_with_the_newest_copy_s_name() {
    let mut cluster = Cluster::default();
    cluster.sources.insert(OWN, (ClusterInfoSource::Own, OWN));
    cluster.records.insert((OWN, 3), record(3, 1, "Own", ClusterInfoSource::Own));
    cluster.copies = vec![
        record(20, 1, "Old", ClusterInfoSource::Copy),
        record(20, 2, "Hall", ClusterInfoSource::Copy),
        record(40, 1, "Porch", ClusterInfoSource::Copy),
    ];
    let expected = vec![
        (3, "Own".to_owned(), ClusterInfoSource::Own),
        (20, "Hall".to_owned(), ClusterInfoSource::Copy),
        (40, "Porch".to_owned(), ClusterInfoSource::Copy),
    ];
    assert_eq!(names(&cluster), expected);
    let single = ClusterView::read(&cluster).record(KIND, 20).map(|record| (record.name, record.source));
    assert_eq!(single, Some(("Hall".to_owned(), ClusterInfoSource::Copy)));
}

#[test]
fn a_record_exists_only_from_its_home_once_the_home_is_held() {
    let mut cluster = Cluster::default();
    cluster.sources.insert(NEIGHBOUR, (ClusterInfoSource::Stale, NEIGHBOUR));
    cluster.copies = vec![record(20, 2, "Hall", ClusterInfoSource::Copy)];
    assert!(names(&cluster).is_empty(), "a held home that lacks the number outweighs any copy");
    assert!(ClusterView::read(&cluster).record(KIND, 20).is_none());
}

fn wire_faults(cluster: &Cluster, node: u8) -> Vec<Option<ClusterWireFaultDto>> {
    ClusterView::read(cluster).node_parts(node, KIND).map(|part| part.wire_fault).collect()
}

#[test]
fn a_part_whose_wire_neighbour_holds_another_record_or_nothing_is_a_wire_fault() {
    let mut cluster = Cluster::default();
    cluster.sources.insert(OWN, (ClusterInfoSource::Own, OWN));
    cluster.sources.insert(NEIGHBOUR, (ClusterInfoSource::Cached, OWN));
    cluster.sources.insert(FAR, (ClusterInfoSource::Cached, FAR));
    cluster.parts.insert((OWN, 0, 1), part(0, 1, 20, 2));
    cluster.parts.insert((OWN, 0, 2), part(0, 2, 21, 1));
    cluster.parts.insert((OWN, 0, 3), part(0, 3, 22, 1));
    cluster.parts.insert((OWN, 1, 4), part(1, 4, 23, 1));
    cluster.parts.insert((NEIGHBOUR, 0, 1), part(0, 1, 20, 2));
    cluster.parts.insert((NEIGHBOUR, 0, 2), part(0, 2, 21, 9));
    cluster.parts.insert((NEIGHBOUR, 0, 3), part(0, 3, 30, 1));
    cluster.parts.insert((FAR, 1, 4), part(1, 4, 50, 1));
    let other = Some(ClusterWireFaultDto { node: NEIGHBOUR, holds: ClusterWireHolding::OtherRecord });
    let nothing = Some(ClusterWireFaultDto { node: NEIGHBOUR, holds: ClusterWireHolding::Nothing });
    assert_eq!(wire_faults(&cluster, OWN), [None, other, other, nothing], "the far node is on another wire");
    let own_fault = Some(ClusterWireFaultDto { node: OWN, holds: ClusterWireHolding::OtherRecord });
    assert_eq!(wire_faults(&cluster, NEIGHBOUR), [None, own_fault, own_fault]);
}

#[test]
fn a_stale_neighbour_proves_no_wire_fault() {
    let mut cluster = Cluster::default();
    cluster.sources.insert(OWN, (ClusterInfoSource::Own, OWN));
    cluster.sources.insert(NEIGHBOUR, (ClusterInfoSource::Stale, OWN));
    cluster.parts.insert((OWN, 0, 1), part(0, 1, 20, 2));
    assert_eq!(wire_faults(&cluster, OWN), [None]);
}
