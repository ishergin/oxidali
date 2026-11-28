use dali2rust_contracts::msg::{
    ClusterBirthMark, ClusterPartLink, ClusterPartUpdateCommand, ClusterRecordKind, ClusterRecordRef,
};
use dali2rust_domain::registry::{
    block_index, is_cluster_record_number, ClusterPartKeys, ClusterPartReadPort, ClusterPartView,
    ClusterRecordConflict, CLUSTER_PART_LOCALS, MAX_ADAPTERS,
};
use dali2rust_platform::slice_store::StoreError;

use super::cluster_settings::ClusterRefusal;
use super::persistence_slices::{
    decode_versioned_slice, PersistableClusterPart, PersistableClusterPartsSlice, CLUSTER_CHANNEL_PARTS_SLICE_VERSION,
    CLUSTER_SCENE_PARTS_SLICE_VERSION,
};
use super::store::{Inner, RegistryStore};

const PART_CAPACITY: usize = MAX_ADAPTERS as usize * CLUSTER_PART_LOCALS as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClusterPartEntry {
    pub adapter: u8,
    pub local: u8,
    pub link: ClusterPartLink,
    pub revision: u32,
}

impl ClusterPartEntry {
    const fn key(&self) -> (u8, u8) {
        (self.adapter, self.local)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClusterPartSet {
    pub entries: Vec<ClusterPartEntry>,
}

impl ClusterPartSet {
    fn preallocated() -> Self {
        Self { entries: Vec::with_capacity(PART_CAPACITY) }
    }

    fn entry(&self, adapter: u8, local: u8) -> Option<&ClusterPartEntry> {
        self.entries.iter().find(|entry| entry.key() == (adapter, local))
    }

    fn keys(&self) -> ClusterPartKeys {
        let mut keys = ClusterPartKeys::default();
        self.entries.iter().for_each(|entry| keys.insert(entry.adapter, entry.local));
        keys
    }

    fn put(&mut self, entry: ClusterPartEntry) {
        self.remove(entry.adapter, entry.local);
        let at = self.entries.partition_point(|held| held.key() < entry.key());
        self.entries.insert(at, entry);
    }

    fn remove(&mut self, adapter: u8, local: u8) {
        self.entries.retain(|held| held.key() != (adapter, local));
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClusterParts {
    pub channels: ClusterPartSet,
    pub scenes: ClusterPartSet,
}

impl ClusterParts {
    pub(crate) fn preallocated() -> Self {
        Self { channels: ClusterPartSet::preallocated(), scenes: ClusterPartSet::preallocated() }
    }

    pub(crate) const fn set(&self, kind: ClusterRecordKind) -> &ClusterPartSet {
        match kind {
            ClusterRecordKind::Channel => &self.channels,
            ClusterRecordKind::Scene => &self.scenes,
        }
    }

    const fn set_mut(&mut self, kind: ClusterRecordKind) -> &mut ClusterPartSet {
        match kind {
            ClusterRecordKind::Channel => &mut self.channels,
            ClusterRecordKind::Scene => &mut self.scenes,
        }
    }

    pub(crate) fn apply(&mut self, edit: PartEdit) -> Option<ClusterRecordKind> {
        match edit {
            PartEdit::Put { kind, entry } => self.set_mut(kind).put(entry),
            PartEdit::Remove { kind, adapter, local } => self.set_mut(kind).remove(adapter, local),
            PartEdit::Unchanged => {}
        }
        edit.kind()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartEdit {
    Put { kind: ClusterRecordKind, entry: ClusterPartEntry },
    Remove { kind: ClusterRecordKind, adapter: u8, local: u8 },
    Unchanged,
}

impl PartEdit {
    const fn kind(self) -> Option<ClusterRecordKind> {
        match self {
            Self::Put { kind, .. } | Self::Remove { kind, .. } => Some(kind),
            Self::Unchanged => None,
        }
    }
}

pub(crate) fn plan_part_edit(inner: &Inner, command: &ClusterPartUpdateCommand) -> Result<PartEdit, ClusterRefusal> {
    let (kind, adapter, local) = (command.kind, command.adapter, command.local);
    let set = inner.cluster_parts.set(kind);
    let held = set.entry(adapter, local);
    let Some(link) = command.link else {
        return held.map(|_| PartEdit::Remove { kind, adapter, local }).ok_or(ClusterRefusal::PartNotFound);
    };
    if !inner.adapter_exists(adapter) || local >= CLUSTER_PART_LOCALS {
        return Err(ClusterRefusal::PartNotFound);
    }
    check_link(inner, kind, &link)?;
    let elsewhere = |other: &&ClusterPartEntry| other.adapter == adapter && other.local != local;
    if set.entries.iter().filter(elsewhere).any(|other| other.link.record.number == link.record.number) {
        return Err(ClusterRefusal::Record(ClusterRecordConflict::RecordLinked));
    }
    linked(kind, adapter, local, held, link)
}

fn check_link(inner: &Inner, kind: ClusterRecordKind, link: &ClusterPartLink) -> Result<(), ClusterRefusal> {
    let record = link.record;
    if !is_cluster_record_number(record.number) || record.mark.generation == 0 {
        return Err(ClusterRefusal::OutOfRange);
    }
    let node = inner.cluster_settings.0.node_number;
    let Some(index) = block_index(node, record.number) else {
        return Ok(());
    };
    let live = inner.cluster_records.block(kind).entry_mark(index);
    if live != Some(record.mark) {
        return Err(ClusterRefusal::Record(ClusterRecordConflict::RecordNotLive));
    }
    Ok(())
}

fn linked(
    kind: ClusterRecordKind,
    adapter: u8,
    local: u8,
    held: Option<&ClusterPartEntry>,
    link: ClusterPartLink,
) -> Result<PartEdit, ClusterRefusal> {
    let revision = match held {
        Some(held) if held.link == link => return Ok(PartEdit::Unchanged),
        Some(held) => held.revision.checked_add(1).ok_or(ClusterRefusal::OutOfRange)?,
        None => 1,
    };
    Ok(PartEdit::Put { kind, entry: ClusterPartEntry { adapter, local, link, revision } })
}

fn part_view(kind: ClusterRecordKind, entry: &ClusterPartEntry) -> ClusterPartView {
    ClusterPartView {
        kind,
        adapter: entry.adapter,
        local: entry.local,
        record: entry.link.record,
        send: entry.link.send,
        accept: entry.link.accept,
        revision: entry.revision,
    }
}

impl ClusterPartReadPort for RegistryStore {
    fn cluster_part(&self, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartView> {
        let g = self.read_inner();
        g.cluster_parts.set(kind).entry(adapter, local).map(|entry| part_view(kind, entry))
    }

    fn cluster_part_keys(&self, kind: ClusterRecordKind) -> ClusterPartKeys {
        self.read_inner().cluster_parts.set(kind).keys()
    }
}

fn persistable_parts(set: &ClusterPartSet) -> PersistableClusterPartsSlice {
    let part = |entry: &ClusterPartEntry| PersistableClusterPart {
        adapter: entry.adapter,
        local: entry.local,
        number: entry.link.record.number,
        generation: entry.link.record.mark.generation,
        nonce: entry.link.record.mark.nonce,
        send: entry.link.send,
        accept: entry.link.accept,
        revision: entry.revision,
    };
    PersistableClusterPartsSlice { parts: set.entries.iter().map(part).collect() }
}

pub(crate) fn persistable_channel_parts(inner: &Inner) -> PersistableClusterPartsSlice {
    persistable_parts(&inner.cluster_parts.channels)
}

pub(crate) fn persistable_scene_parts(inner: &Inner) -> PersistableClusterPartsSlice {
    persistable_parts(&inner.cluster_parts.scenes)
}

fn stored_entry(part: &PersistableClusterPart) -> Option<ClusterPartEntry> {
    let fits = part.adapter < MAX_ADAPTERS && part.local < CLUSTER_PART_LOCALS;
    let names = is_cluster_record_number(part.number) && part.generation >= 1 && part.revision >= 1;
    let mark = ClusterBirthMark { generation: part.generation, nonce: part.nonce };
    let record = ClusterRecordRef { number: part.number, mark };
    let link = ClusterPartLink { record, send: part.send, accept: part.accept };
    let entry = ClusterPartEntry { adapter: part.adapter, local: part.local, link, revision: part.revision };
    (fits && names).then_some(entry)
}

pub(crate) fn stored_parts(slice: &PersistableClusterPartsSlice) -> Option<ClusterPartSet> {
    let mut set = ClusterPartSet::preallocated();
    for part in &slice.parts {
        let entry = stored_entry(part)?;
        let later = set.entries.last().is_none_or(|last| last.key() < entry.key());
        let same_record = |held: &ClusterPartEntry| held.link.record.number == entry.link.record.number;
        let twice = set.entries.iter().filter(|held| held.adapter == entry.adapter).any(same_record);
        if !later || twice {
            return None;
        }
        set.entries.push(entry);
    }
    Some(set)
}

fn decode_parts(bytes: &[u8], version: u32, what: &str) -> Result<PersistableClusterPartsSlice, StoreError> {
    let slice = decode_versioned_slice::<PersistableClusterPartsSlice>(bytes, version)
        .map_err(|e| StoreError::Backend(e.to_string()))?;
    stored_parts(&slice).ok_or_else(|| StoreError::Backend(format!("{what} refused: cluster_parts_inconsistent")))?;
    Ok(slice)
}

pub(crate) fn decode_channel_parts(bytes: &[u8]) -> Result<PersistableClusterPartsSlice, StoreError> {
    decode_parts(bytes, CLUSTER_CHANNEL_PARTS_SLICE_VERSION, "cluster channel parts")
}

pub(crate) fn decode_scene_parts(bytes: &[u8]) -> Result<PersistableClusterPartsSlice, StoreError> {
    decode_parts(bytes, CLUSTER_SCENE_PARTS_SLICE_VERSION, "cluster scene parts")
}

fn hydrate_parts(inner: &mut Inner, kind: ClusterRecordKind, slice: &PersistableClusterPartsSlice) {
    *inner.cluster_parts.set_mut(kind) = stored_parts(slice).unwrap_or_else(ClusterPartSet::preallocated);
    super::cluster_copies::prune_copies(inner, kind);
}

pub(crate) fn hydrate_channel_parts_inner(inner: &mut Inner, slice: &PersistableClusterPartsSlice) {
    hydrate_parts(inner, ClusterRecordKind::Channel, slice);
}

pub(crate) fn hydrate_scene_parts_inner(inner: &mut Inner, slice: &PersistableClusterPartsSlice) {
    hydrate_parts(inner, ClusterRecordKind::Scene, slice);
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::{
        fixed_text_64, ClusterForwarding, ClusterRecordAction, ClusterRecordUpdateCommand, ClusterSettingsField,
        ClusterSettingsUpdateCommand, ClusterTextField, ClusterTextUpdateCommand,
    };
    use dali2rust_domain::registry::ClusterSettingsReadPort;

    use super::super::cluster_settings::ClusterPatch;
    use super::super::persistence_slices::{encode_persistence_blob, PersistenceEnvelope};
    use super::*;

    const SERIES: u64 = 13;
    const CHANNEL: ClusterRecordKind = ClusterRecordKind::Channel;
    const SCENE: ClusterRecordKind = ClusterRecordKind::Scene;
    const FOREIGN: u8 = 20;
    const OWN_NONCE: u32 = 0x0bad_cafe;

    fn reference(number: u8, generation: u32, nonce: u32) -> ClusterRecordRef {
        ClusterRecordRef { number, mark: ClusterBirthMark { generation, nonce } }
    }

    fn link(record: ClusterRecordRef, send: bool) -> ClusterPartLink {
        ClusterPartLink { record, send, accept: true }
    }

    fn part(kind: ClusterRecordKind, adapter: u8, local: u8, link: Option<ClusterPartLink>) -> ClusterPatch {
        ClusterPatch::Part(ClusterPartUpdateCommand { series: SERIES, kind, adapter, local, link })
    }

    fn commit(store: &RegistryStore, patches: Vec<ClusterPatch>) -> Result<(), ClusterRefusal> {
        let frames = u8::try_from(patches.len()).expect("frames");
        for patch in patches {
            store.stage_cluster_patch(SERIES, patch)?;
        }
        store.commit_cluster_settings(SERIES, frames).map(|_| ())
    }

    fn put(
        store: &RegistryStore,
        kind: ClusterRecordKind,
        adapter: u8,
        local: u8,
        link: ClusterPartLink,
    ) -> Result<(), ClusterRefusal> {
        commit(store, vec![part(kind, adapter, local, Some(link))])
    }

    fn revision(store: &RegistryStore, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<u32> {
        store.cluster_part(kind, adapter, local).map(|view| view.revision)
    }

    fn own_record(store: &RegistryStore, kind: ClusterRecordKind, number: u8) -> ClusterRecordRef {
        let action = ClusterRecordAction::Create { number: Some(number), nonce: OWN_NONCE };
        let create = ClusterPatch::Record(ClusterRecordUpdateCommand { series: SERIES, kind, action });
        commit(store, vec![create]).expect("record created");
        reference(number, 1, OWN_NONCE)
    }

    #[test]
    fn a_record_of_another_node_is_kept_as_given_and_its_part_starts_at_revision_one() {
        let store = RegistryStore::with_adapter_count(1);
        let given = link(reference(FOREIGN, 4, 77), false);
        put(&store, CHANNEL, 0, 3, given).expect("linked");
        let (record, send, accept, revision) = (given.record, false, true, 1);
        let expected = ClusterPartView { kind: CHANNEL, adapter: 0, local: 3, record, send, accept, revision };
        assert_eq!(store.cluster_part(CHANNEL, 0, 3), Some(expected));
        assert_eq!(store.cluster_part(SCENE, 0, 3), None);
    }

    #[test]
    fn a_record_of_this_node_is_linked_only_with_its_live_mark() {
        let store = RegistryStore::with_adapter_count(1);
        let live = own_record(&store, SCENE, 2);
        let not_live = Err(ClusterRefusal::Record(ClusterRecordConflict::RecordNotLive));
        for stale in [reference(2, 2, OWN_NONCE), reference(2, 1, OWN_NONCE + 1), reference(5, 1, OWN_NONCE)] {
            assert_eq!(put(&store, SCENE, 0, 4, link(stale, true)), not_live, "{stale:?}");
        }
        assert_eq!(put(&store, CHANNEL, 0, 4, link(live, true)), not_live, "the channel block holds no record 2");
        put(&store, SCENE, 0, 4, link(live, true)).expect("the live mark links");
        assert_eq!(revision(&store, SCENE, 0, 4), Some(1));
    }

    fn node_number(node: u8) -> ClusterPatch {
        let mut body = ClusterSettingsUpdateCommand {
            series: SERIES,
            patch_mask: 0,
            enabled: false,
            node_number: node,
            port: 0,
            wire_same_as: None,
            forwarding: ClusterForwarding::FACTORY,
            accept_all_lamps: false,
            send_all_lamps: false,
            accept_single_lamps: false,
            wire_protection: true,
        };
        body.patch(ClusterSettingsField::NodeNumber);
        ClusterPatch::Settings(body)
    }

    #[test]
    fn the_own_block_follows_the_node_number() {
        let store = RegistryStore::with_adapter_count(1);
        commit(&store, vec![node_number(1)]).expect("node 1");
        let not_live = Err(ClusterRefusal::Record(ClusterRecordConflict::RecordNotLive));
        assert_eq!(put(&store, CHANNEL, 0, 3, link(reference(FOREIGN, 1, 5), true)), not_live, "20 is node 1's own");
        put(&store, CHANNEL, 0, 4, link(reference(2, 1, 5), true)).expect("2 is node 0's, kept as given");
        let live = own_record(&store, CHANNEL, FOREIGN);
        put(&store, CHANNEL, 0, 3, link(live, true)).expect("the live record 20 links");
    }

    #[test]
    fn a_part_of_an_adapter_the_node_no_longer_has_is_read_and_removed_but_not_written() {
        let store = RegistryStore::with_adapter_count(1);
        let slice = PersistableClusterPartsSlice { parts: vec![stored(1, 2, FOREIGN)] };
        hydrate_channel_parts_inner(&mut store.write_inner(), &slice);
        assert_eq!(revision(&store, CHANNEL, 1, 2), Some(3));
        let changed = put(&store, CHANNEL, 1, 2, link(reference(FOREIGN, 2, 9), false));
        assert_eq!(changed, Err(ClusterRefusal::PartNotFound));
        commit(&store, vec![part(CHANNEL, 1, 2, None)]).expect("removed");
        assert_eq!(store.cluster_part(CHANNEL, 1, 2), None);
    }

    #[test]
    fn a_changed_part_raises_its_revision_and_the_same_part_keeps_it_and_writes_nothing() {
        let store = RegistryStore::with_adapter_count(1);
        let first = link(reference(FOREIGN, 1, 5), true);
        put(&store, CHANNEL, 0, 3, first).expect("linked");
        assert!(store.dirty.take_cluster_channel_parts_dirty());
        put(&store, CHANNEL, 0, 3, first).expect("written again");
        assert_eq!(revision(&store, CHANNEL, 0, 3), Some(1));
        assert!(!store.dirty.take_cluster_channel_parts_dirty(), "the same part is no write");
        put(&store, CHANNEL, 0, 3, link(first.record, false)).expect("changed");
        assert_eq!(revision(&store, CHANNEL, 0, 3), Some(2));
        assert!(store.dirty.take_cluster_channel_parts_dirty());
        assert!(!store.dirty.take_cluster_scene_parts_dirty(), "a channel part is no scene-part write");
    }

    #[test]
    fn one_record_has_one_part_on_each_adapter_and_its_own_part_may_change() {
        let store = RegistryStore::with_adapter_count(2);
        let record = reference(FOREIGN, 1, 5);
        put(&store, CHANNEL, 0, 3, link(record, true)).expect("linked");
        let linked = Err(ClusterRefusal::Record(ClusterRecordConflict::RecordLinked));
        assert_eq!(put(&store, CHANNEL, 0, 4, link(record, true)), linked);
        put(&store, CHANNEL, 0, 3, link(record, false)).expect("the part that holds the record changes it");
        put(&store, CHANNEL, 1, 4, link(record, true)).expect("another adapter has a part of its own");
        put(&store, SCENE, 0, 4, link(record, true)).expect("a scene part is another kind");
        assert_eq!(store.cluster_part_keys(CHANNEL).iter().collect::<Vec<_>>(), [(0, 3), (1, 4)]);
    }

    #[test]
    fn a_part_of_no_adapter_or_no_group_is_not_found_and_so_is_removing_none() {
        let store = RegistryStore::with_adapter_count(1);
        let any = link(reference(FOREIGN, 1, 5), true);
        assert_eq!(put(&store, CHANNEL, 1, 3, any), Err(ClusterRefusal::PartNotFound));
        assert_eq!(put(&store, CHANNEL, 0, CLUSTER_PART_LOCALS, any), Err(ClusterRefusal::PartNotFound));
        assert_eq!(commit(&store, vec![part(CHANNEL, 0, 3, None)]), Err(ClusterRefusal::PartNotFound));
        put(&store, CHANNEL, 0, 3, any).expect("linked");
        commit(&store, vec![part(CHANNEL, 0, 3, None)]).expect("removed");
        assert_eq!(store.cluster_part(CHANNEL, 0, 3), None);
    }

    #[test]
    fn a_record_number_past_the_last_or_a_generation_of_zero_is_out_of_range() {
        let store = RegistryStore::with_adapter_count(1);
        for record in [reference(128, 1, 5), reference(FOREIGN, 0, 5), reference(2, 0, 0)] {
            let refused = put(&store, CHANNEL, 0, 3, link(record, true));
            assert_eq!(refused, Err(ClusterRefusal::OutOfRange), "{record:?}");
        }
        assert_eq!(store.cluster_part(CHANNEL, 0, 3), None);
    }

    #[test]
    fn a_part_is_the_only_frame_of_its_series() {
        let store = RegistryStore::with_adapter_count(1);
        let name = ClusterPatch::Text(ClusterTextUpdateCommand {
            series: SERIES,
            field: ClusterTextField::ClusterName,
            text: fixed_text_64("dali-cluster"),
        });
        let refused = commit(&store, vec![part(CHANNEL, 0, 3, Some(link(reference(FOREIGN, 1, 5), true))), name]);
        assert_eq!(refused, Err(ClusterRefusal::OutOfRange));
        assert_eq!(store.cluster_part(CHANNEL, 0, 3), None);
        assert_eq!(store.cluster_settings_view().cluster_name, "");
    }

    fn stored(adapter: u8, local: u8, number: u8) -> PersistableClusterPart {
        let (generation, nonce, revision) = (2, 9, 3);
        PersistableClusterPart { adapter, local, number, generation, nonce, send: true, accept: false, revision }
    }

    #[test]
    fn the_stored_slice_restores_every_field() {
        let store = RegistryStore::with_adapter_count(2);
        put(&store, SCENE, 1, 15, link(reference(127, u32::MAX, u32::MAX), false)).expect("linked");
        put(&store, SCENE, 0, 2, link(reference(FOREIGN, 3, 4), true)).expect("linked");
        let slice = persistable_scene_parts(&store.read_inner());
        let envelope = PersistenceEnvelope { version: CLUSTER_SCENE_PARTS_SLICE_VERSION, data: slice.clone() };
        let decoded = decode_scene_parts(&encode_persistence_blob(&envelope).expect("encode"));
        assert_eq!(decoded.expect("decoded").parts, slice.parts);
        let mut restored = Inner::default();
        hydrate_scene_parts_inner(&mut restored, &slice);
        assert_eq!(restored.cluster_parts.scenes, store.read_inner().cluster_parts.scenes);
        assert_eq!(restored.cluster_parts.channels, ClusterPartSet::default());
    }

    type Break = fn(&mut Vec<PersistableClusterPart>);

    const BREAKS: [(&str, Break); 8] = [
        ("adapter past the last", |parts| parts[0].adapter = MAX_ADAPTERS),
        ("group past the last", |parts| parts[0].local = CLUSTER_PART_LOCALS),
        ("record number past the last", |parts| parts[0].number = 128),
        ("generation zero", |parts| parts[0].generation = 0),
        ("revision zero", |parts| parts[0].revision = 0),
        ("out of order", |parts| parts.swap(0, 1)),
        ("one key twice", |parts| parts[1].local = parts[0].local),
        ("one record twice on an adapter", |parts| parts[1].number = parts[0].number),
    ];

    #[test]
    fn a_slice_that_breaks_a_rule_does_not_decode() {
        let held = vec![stored(0, 1, 30), stored(0, 2, 31), stored(1, 0, 30)];
        let kept = stored_parts(&PersistableClusterPartsSlice { parts: held.clone() });
        assert!(kept.is_some(), "one record on two adapters");
        for (case, break_it) in BREAKS {
            let mut parts = held.clone();
            break_it(&mut parts);
            let data = PersistableClusterPartsSlice { parts };
            let envelope = PersistenceEnvelope { version: CLUSTER_CHANNEL_PARTS_SLICE_VERSION, data };
            let bytes = encode_persistence_blob(&envelope).expect("encode");
            assert!(decode_channel_parts(&bytes).is_err(), "{case}");
        }
    }

    #[test]
    fn the_widest_parts_slice_fits_its_slot() {
        let widest = |key: usize| PersistableClusterPart {
            adapter: u8::try_from(key / usize::from(CLUSTER_PART_LOCALS)).expect("adapter"),
            local: u8::try_from(key % usize::from(CLUSTER_PART_LOCALS)).expect("local"),
            number: u8::MAX,
            generation: u32::MAX,
            nonce: u32::MAX,
            send: true,
            accept: true,
            revision: u32::MAX,
        };
        let data = PersistableClusterPartsSlice { parts: (0..PART_CAPACITY).map(widest).collect() };
        let bytes = encode_persistence_blob(&PersistenceEnvelope { version: u32::MAX, data }).expect("encode");
        let payload = dali2rust_bsp::slice_layout::SMALL_SLOT_PAYLOAD_BYTES;
        assert!(bytes.len() <= usize::try_from(payload).expect("payload"), "{} B", bytes.len());
    }
}
