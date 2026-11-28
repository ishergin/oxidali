use dali2rust_contracts::msg::{
    fixed_text_64, ClusterBirthMark, ClusterCopyAction, ClusterRecordCopyCommand, ClusterRecordKind, ClusterRecordRef,
    FixedText64,
};
use dali2rust_domain::registry::{
    block_index, cluster_node_of, is_cluster_record_number, ClusterCopyReadPort, ClusterRecordView,
    MAX_CLUSTER_COPIES_PER_KIND, MAX_CLUSTER_RECORD_NAME_BYTES,
};
use dali2rust_platform::slice_store::StoreError;

use super::cluster_settings::{ClusterRefusal, ClusterSettingsRecord};
use super::persistence_slices::{
    decode_versioned_slice, PersistableClusterCopiesSlice, PersistableClusterCopy,
    CLUSTER_CHANNEL_COPIES_SLICE_VERSION, CLUSTER_SCENE_COPIES_SLICE_VERSION,
};
use super::store::{Inner, RegistryStore};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClusterCopyEntry {
    pub record: ClusterRecordRef,
    pub name: FixedText64,
    pub revision: u32,
}

impl ClusterCopyEntry {
    const fn key(&self) -> (u8, u32, u32) {
        key_of(self.record)
    }
}

const fn key_of(record: ClusterRecordRef) -> (u8, u32, u32) {
    (record.number, record.mark.generation, record.mark.nonce)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClusterCopySet {
    pub entries: Vec<ClusterCopyEntry>,
}

impl ClusterCopySet {
    fn preallocated() -> Self {
        Self { entries: Vec::with_capacity(MAX_CLUSTER_COPIES_PER_KIND) }
    }

    fn entry(&self, record: ClusterRecordRef) -> Option<&ClusterCopyEntry> {
        self.entries.iter().find(|entry| entry.record == record)
    }

    fn put(&mut self, entry: ClusterCopyEntry) {
        self.entries.retain(|held| held.record != entry.record);
        let at = self.entries.partition_point(|held| held.key() < entry.key());
        self.entries.insert(at, entry);
    }

    fn retain(&mut self, keep: impl Fn(&ClusterCopyEntry) -> bool) -> bool {
        let before = self.entries.len();
        self.entries.retain(keep);
        self.entries.len() != before
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClusterCopies {
    pub channels: ClusterCopySet,
    pub scenes: ClusterCopySet,
}

impl ClusterCopies {
    pub(crate) fn preallocated() -> Self {
        Self { channels: ClusterCopySet::preallocated(), scenes: ClusterCopySet::preallocated() }
    }

    pub(crate) const fn set(&self, kind: ClusterRecordKind) -> &ClusterCopySet {
        match kind {
            ClusterRecordKind::Channel => &self.channels,
            ClusterRecordKind::Scene => &self.scenes,
        }
    }

    const fn set_mut(&mut self, kind: ClusterRecordKind) -> &mut ClusterCopySet {
        match kind {
            ClusterRecordKind::Channel => &mut self.channels,
            ClusterRecordKind::Scene => &mut self.scenes,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopyOutcome {
    Stored,
    Dropped,
    Unchanged,
}

fn names(inner: &Inner, kind: ClusterRecordKind, record: ClusterRecordRef) -> bool {
    inner.cluster_parts.set(kind).entries.iter().any(|part| part.link.record == record)
}

fn foreign(inner: &Inner, record: ClusterRecordRef) -> bool {
    let node = inner.cluster_settings.0.node_number;
    is_cluster_record_number(record.number) && block_index(node, record.number).is_none()
}

fn plan_copy(inner: &Inner, command: &ClusterRecordCopyCommand) -> (CopyOutcome, Option<ClusterCopyEntry>) {
    let (kind, record) = (command.kind, command.record);
    let held = inner.cluster_copies.set(kind).entry(record);
    if !foreign(inner, record) || record.mark.generation == 0 {
        return (CopyOutcome::Unchanged, None);
    }
    match &command.action {
        ClusterCopyAction::Drop if held.is_some() => (CopyOutcome::Dropped, None),
        ClusterCopyAction::Drop => (CopyOutcome::Unchanged, None),
        ClusterCopyAction::Store { revision, name } => {
            let newer = held.is_none_or(|held| held.revision < *revision);
            if *revision == 0 || !newer || !names(inner, kind, record) {
                return (CopyOutcome::Unchanged, None);
            }
            (CopyOutcome::Stored, Some(ClusterCopyEntry { record, name: name.clone(), revision: *revision }))
        }
    }
}

pub(crate) fn prune_copies(inner: &mut Inner, kind: ClusterRecordKind) -> bool {
    let node = inner.cluster_settings.0.node_number;
    let parts = &inner.cluster_parts.set(kind).entries;
    let named = |copy: &ClusterCopyEntry| parts.iter().any(|part| part.link.record == copy.record);
    let outside = |copy: &ClusterCopyEntry| block_index(node, copy.record.number).is_none();
    inner.cluster_copies.set_mut(kind).retain(|copy| named(copy) && outside(copy))
}

impl RegistryStore {
    pub(crate) fn apply_cluster_copy(
        &self,
        command: &ClusterRecordCopyCommand,
    ) -> Result<(CopyOutcome, ClusterSettingsRecord), ClusterRefusal> {
        let mut g = self.write_inner();
        if !g.dali_settings.application_active {
            return Err(ClusterRefusal::Standby);
        }
        let (outcome, entry) = plan_copy(&g, command);
        let record = g.cluster_settings.clone();
        let set = g.cluster_copies.set_mut(command.kind);
        match (outcome, entry) {
            (CopyOutcome::Stored, Some(entry)) => set.put(entry),
            (CopyOutcome::Dropped, _) => {
                set.retain(|held| held.record != command.record);
            }
            _ => return Ok((CopyOutcome::Unchanged, record)),
        }
        drop(g);
        self.mark_cluster_copies_dirty(command.kind);
        Ok((outcome, record))
    }

    pub(crate) fn mark_cluster_copies_dirty(&self, kind: ClusterRecordKind) {
        match kind {
            ClusterRecordKind::Channel => self.dirty.mark_cluster_channel_copies_dirty(),
            ClusterRecordKind::Scene => self.dirty.mark_cluster_scene_copies_dirty(),
        }
    }
}

fn copy_view(kind: ClusterRecordKind, entry: &ClusterCopyEntry) -> ClusterRecordView {
    ClusterRecordView {
        kind,
        number: entry.record.number,
        home_node: cluster_node_of(entry.record.number),
        mark: entry.record.mark,
        name: entry.name.as_str().to_string(),
        revision: entry.revision,
    }
}

impl ClusterCopyReadPort for RegistryStore {
    fn cluster_copy(&self, kind: ClusterRecordKind, record: ClusterRecordRef) -> Option<ClusterRecordView> {
        let g = self.read_inner();
        g.cluster_copies.set(kind).entry(record).map(|entry| copy_view(kind, entry))
    }

    fn cluster_copy_at(&self, kind: ClusterRecordKind, index: usize) -> Option<ClusterRecordView> {
        let g = self.read_inner();
        g.cluster_copies.set(kind).entries.get(index).map(|entry| copy_view(kind, entry))
    }
}

fn persistable_copies(set: &ClusterCopySet) -> PersistableClusterCopiesSlice {
    let copy = |entry: &ClusterCopyEntry| PersistableClusterCopy {
        number: entry.record.number,
        generation: entry.record.mark.generation,
        nonce: entry.record.mark.nonce,
        name: entry.name.as_str().to_string(),
        revision: entry.revision,
    };
    PersistableClusterCopiesSlice { copies: set.entries.iter().map(copy).collect() }
}

pub(crate) fn persistable_channel_copies(inner: &Inner) -> PersistableClusterCopiesSlice {
    persistable_copies(&inner.cluster_copies.channels)
}

pub(crate) fn persistable_scene_copies(inner: &Inner) -> PersistableClusterCopiesSlice {
    persistable_copies(&inner.cluster_copies.scenes)
}

fn stored_entry(copy: &PersistableClusterCopy) -> Option<ClusterCopyEntry> {
    let names = is_cluster_record_number(copy.number) && copy.generation >= 1 && copy.revision >= 1;
    let fits = copy.name.len() <= MAX_CLUSTER_RECORD_NAME_BYTES;
    let record = ClusterRecordRef { number: copy.number, mark: ClusterBirthMark { generation: copy.generation, nonce: copy.nonce } };
    (names && fits).then(|| ClusterCopyEntry { record, name: fixed_text_64(&copy.name), revision: copy.revision })
}

pub(crate) fn stored_copies(slice: &PersistableClusterCopiesSlice) -> Option<ClusterCopySet> {
    if slice.copies.len() > MAX_CLUSTER_COPIES_PER_KIND {
        return None;
    }
    let mut set = ClusterCopySet::preallocated();
    for copy in &slice.copies {
        let entry = stored_entry(copy)?;
        if set.entries.last().is_some_and(|last| last.key() >= entry.key()) {
            return None;
        }
        set.entries.push(entry);
    }
    Some(set)
}

fn decode_copies(bytes: &[u8], version: u32, what: &str) -> Result<PersistableClusterCopiesSlice, StoreError> {
    let slice = decode_versioned_slice::<PersistableClusterCopiesSlice>(bytes, version)
        .map_err(|e| StoreError::Backend(e.to_string()))?;
    stored_copies(&slice).ok_or_else(|| StoreError::Backend(format!("{what} refused: cluster_copies_inconsistent")))?;
    Ok(slice)
}

pub(crate) fn decode_channel_copies(bytes: &[u8]) -> Result<PersistableClusterCopiesSlice, StoreError> {
    decode_copies(bytes, CLUSTER_CHANNEL_COPIES_SLICE_VERSION, "cluster channel copies")
}

pub(crate) fn decode_scene_copies(bytes: &[u8]) -> Result<PersistableClusterCopiesSlice, StoreError> {
    decode_copies(bytes, CLUSTER_SCENE_COPIES_SLICE_VERSION, "cluster scene copies")
}

fn hydrate_copies(inner: &mut Inner, kind: ClusterRecordKind, slice: &PersistableClusterCopiesSlice) {
    *inner.cluster_copies.set_mut(kind) = stored_copies(slice).unwrap_or_else(ClusterCopySet::preallocated);
    prune_copies(inner, kind);
}

pub(crate) fn hydrate_channel_copies_inner(inner: &mut Inner, slice: &PersistableClusterCopiesSlice) {
    hydrate_copies(inner, ClusterRecordKind::Channel, slice);
}

pub(crate) fn hydrate_scene_copies_inner(inner: &mut Inner, slice: &PersistableClusterCopiesSlice) {
    hydrate_copies(inner, ClusterRecordKind::Scene, slice);
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::{ClusterPartLink, ClusterPartUpdateCommand};
    use dali2rust_domain::registry::MAX_CLUSTER_PARTS_PER_KIND;

    use super::super::cluster_settings::ClusterPatch;
    use super::super::persistence_slices::{encode_persistence_blob, PersistenceEnvelope};
    use super::*;

    const SERIES: u64 = 17;
    const CHANNEL: ClusterRecordKind = ClusterRecordKind::Channel;
    const SCENE: ClusterRecordKind = ClusterRecordKind::Scene;
    const FOREIGN: u8 = 20;

    fn reference(number: u8, generation: u32, nonce: u32) -> ClusterRecordRef {
        ClusterRecordRef { number, mark: ClusterBirthMark { generation, nonce } }
    }

    fn commit(store: &RegistryStore, patch: ClusterPatch) {
        store.stage_cluster_patch(SERIES, patch).expect("staged");
        store.commit_cluster_settings(SERIES, 1).expect("committed");
    }

    fn link_on(registry: &RegistryStore, adapter: u8, local: u8, record: Option<ClusterRecordRef>) {
        let link = record.map(|record| ClusterPartLink { record, send: true, accept: true });
        let kind = CHANNEL;
        commit(registry, ClusterPatch::Part(ClusterPartUpdateCommand { series: SERIES, kind, adapter, local, link }));
    }

    fn link(registry: &RegistryStore, local: u8, record: Option<ClusterRecordRef>) {
        link_on(registry, 0, local, record);
    }

    fn store(kind: ClusterRecordKind, record: ClusterRecordRef, revision: u32, name: &str) -> ClusterRecordCopyCommand {
        let action = ClusterCopyAction::Store { revision, name: fixed_text_64(name) };
        ClusterRecordCopyCommand { kind, record, action }
    }

    fn held(registry: &RegistryStore, kind: ClusterRecordKind, record: ClusterRecordRef) -> Option<(String, u32)> {
        registry.cluster_copy(kind, record).map(|view| (view.name, view.revision))
    }

    fn linked_store(record: ClusterRecordRef) -> RegistryStore {
        let registry = RegistryStore::with_adapter_count(2);
        link(&registry, 3, Some(record));
        registry
    }

    #[test]
    fn a_copy_is_kept_only_for_a_record_a_part_names() {
        let named = reference(FOREIGN, 2, 9);
        let registry = linked_store(named);
        let unnamed = reference(FOREIGN, 3, 9);
        let outcome = |command| registry.apply_cluster_copy(&command).map(|(outcome, _)| outcome);
        assert_eq!(outcome(store(CHANNEL, unnamed, 1, "Hall")), Ok(CopyOutcome::Unchanged));
        assert_eq!(outcome(store(SCENE, named, 1, "Hall")), Ok(CopyOutcome::Unchanged), "a part of the other kind");
        assert_eq!(outcome(store(CHANNEL, named, 1, "Hall")), Ok(CopyOutcome::Stored));
        let view = registry.cluster_copy(CHANNEL, named).expect("held");
        assert_eq!((view.home_node, view.mark, view.name.as_str(), view.revision), (1, named.mark, "Hall", 1));
        assert_eq!(held(&registry, CHANNEL, unnamed), None);
    }

    #[test]
    fn only_a_higher_revision_of_the_same_mark_replaces_a_copy() {
        let record = reference(FOREIGN, 2, 9);
        let registry = linked_store(record);
        registry.apply_cluster_copy(&store(CHANNEL, record, 4, "Hall")).expect("stored");
        for (revision, name) in [(4, "Equal"), (3, "Lower"), (0, "Zero")] {
            let (outcome, _) = registry.apply_cluster_copy(&store(CHANNEL, record, revision, name)).expect("answered");
            assert_eq!(outcome, CopyOutcome::Unchanged, "{name}");
        }
        assert_eq!(held(&registry, CHANNEL, record), Some(("Hall".to_owned(), 4)));
        registry.apply_cluster_copy(&store(CHANNEL, record, 5, "Corridor")).expect("stored");
        assert_eq!(held(&registry, CHANNEL, record), Some(("Corridor".to_owned(), 5)));
    }

    #[test]
    fn another_mark_at_the_number_is_a_copy_of_its_own_and_needs_its_own_part() {
        let old = reference(FOREIGN, 2, 9);
        let registry = linked_store(old);
        registry.apply_cluster_copy(&store(CHANNEL, old, 1, "Old")).expect("stored");
        let reissued = reference(FOREIGN, 3, 4);
        registry.apply_cluster_copy(&store(CHANNEL, reissued, 7, "New")).expect("answered");
        assert_eq!(held(&registry, CHANNEL, reissued), None, "no part names the new mark");
        assert_eq!(held(&registry, CHANNEL, old), Some(("Old".to_owned(), 1)));
        link(&registry, 3, Some(reissued));
        assert_eq!(held(&registry, CHANNEL, old), None, "the old copy goes with its last part");
        registry.apply_cluster_copy(&store(CHANNEL, reissued, 7, "New")).expect("stored");
        assert_eq!(held(&registry, CHANNEL, reissued), Some(("New".to_owned(), 7)));
    }

    #[test]
    fn a_copy_goes_with_its_last_part_and_by_a_drop() {
        let record = reference(FOREIGN, 2, 9);
        let registry = linked_store(record);
        link_on(&registry, 1, 4, Some(record));
        registry.apply_cluster_copy(&store(CHANNEL, record, 1, "Hall")).expect("stored");
        link(&registry, 3, None);
        assert!(held(&registry, CHANNEL, record).is_some(), "a part still names it");
        let drop = ClusterRecordCopyCommand { kind: CHANNEL, record, action: ClusterCopyAction::Drop };
        assert_eq!(registry.apply_cluster_copy(&drop).map(|(outcome, _)| outcome), Ok(CopyOutcome::Dropped));
        assert_eq!(held(&registry, CHANNEL, record), None);
        registry.apply_cluster_copy(&store(CHANNEL, record, 1, "Hall")).expect("stored");
        link_on(&registry, 1, 4, None);
        assert_eq!(held(&registry, CHANNEL, record), None, "no part is left");
    }

    #[test]
    fn a_copy_of_a_number_of_this_node_or_past_the_cluster_is_never_kept() {
        let own = reference(3, 1, 9);
        let registry = RegistryStore::with_adapter_count(1);
        registry.write_inner().cluster_parts.channels.entries.clear();
        for record in [own, reference(200, 1, 9), reference(FOREIGN, 0, 9)] {
            let (outcome, _) = registry.apply_cluster_copy(&store(CHANNEL, record, 1, "X")).expect("answered");
            assert_eq!(outcome, CopyOutcome::Unchanged, "{record:?}");
        }
    }

    #[test]
    fn the_standby_keeps_no_copy() {
        let record = reference(FOREIGN, 2, 9);
        let registry = linked_store(record);
        registry.write_inner().dali_settings.application_active = false;
        assert_eq!(registry.apply_cluster_copy(&store(CHANNEL, record, 1, "Hall")), Err(ClusterRefusal::Standby));
        assert_eq!(held(&registry, CHANNEL, record), None);
    }

    fn encoded(slice: &PersistableClusterCopiesSlice) -> Vec<u8> {
        let envelope = PersistenceEnvelope { version: CLUSTER_CHANNEL_COPIES_SLICE_VERSION, data: slice.clone() };
        encode_persistence_blob(&envelope).expect("encode")
    }

    #[test]
    fn copies_survive_their_slice_and_a_copy_no_part_names_is_not_restored() {
        let kept = reference(FOREIGN, 2, 9);
        let registry = linked_store(kept);
        registry.apply_cluster_copy(&store(CHANNEL, kept, 3, "Hall")).expect("stored");
        let mut slice = persistable_channel_copies(&registry.read_inner());
        slice.copies.push(PersistableClusterCopy { number: 40, generation: 1, nonce: 1, name: "Gone".into(), revision: 1 });
        let decoded = decode_channel_copies(&encoded(&slice)).expect("decodes");
        let restored = linked_store(kept);
        hydrate_channel_copies_inner(&mut restored.write_inner(), &decoded);
        assert_eq!(held(&restored, CHANNEL, kept), Some(("Hall".to_owned(), 3)));
        assert_eq!(restored.read_inner().cluster_copies.channels.entries.len(), 1);
    }

    type Break = fn(&mut PersistableClusterCopiesSlice);

    const BREAKS: [(&str, Break); 6] = [
        ("a number past the cluster", |s| s.copies[0].number = 128),
        ("generation zero", |s| s.copies[0].generation = 0),
        ("revision zero", |s| s.copies[0].revision = 0),
        ("a name too long", |s| s.copies[0].name = "n".repeat(MAX_CLUSTER_RECORD_NAME_BYTES + 1)),
        ("a copy twice", |s| s.copies.push(s.copies[0].clone())),
        ("out of order", |s| s.copies.reverse()),
    ];

    fn two_copies() -> PersistableClusterCopiesSlice {
        let copy = |number| PersistableClusterCopy { number, generation: 1, nonce: 1, name: "C".into(), revision: 1 };
        PersistableClusterCopiesSlice { copies: vec![copy(20), copy(40)] }
    }

    #[test]
    fn a_copies_slice_that_disagrees_with_itself_does_not_decode() {
        assert!(decode_channel_copies(&encoded(&two_copies())).is_ok(), "the unbroken slice decodes");
        for (what, break_it) in BREAKS {
            let mut broken = two_copies();
            break_it(&mut broken);
            assert!(decode_channel_copies(&encoded(&broken)).is_err(), "{what}");
        }
    }

    #[test]
    fn the_widest_copies_slice_fits_its_slot() {
        let widest = PersistableClusterCopy {
            number: 127,
            generation: u32::MAX,
            nonce: u32::MAX,
            name: "\u{10FFFF}".repeat(MAX_CLUSTER_RECORD_NAME_BYTES / 4),
            revision: u32::MAX,
        };
        let slice = PersistableClusterCopiesSlice { copies: vec![widest; MAX_CLUSTER_PARTS_PER_KIND] };
        let bytes = encoded(&slice).len();
        let payload = dali2rust_bsp::slice_layout::COPY_SLOT_PAYLOAD_BYTES as usize;
        assert!(bytes <= payload, "{bytes} B of copies past the {payload} B slot");
        assert!(bytes > dali2rust_bsp::slice_layout::SMALL_SLOT_PAYLOAD_BYTES as usize, "a small slot would not do");
    }
}
