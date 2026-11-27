use dali2rust_contracts::msg::{
    ClusterBirthMark, ClusterRecordAction, ClusterRecordKind, ClusterRecordUpdateCommand, FixedText64,
};
use dali2rust_domain::registry::{
    block_index, cluster_block, default_record_name, is_cluster_record_number, next_record_index,
    ClusterRecordConflict, ClusterRecordNumbers, ClusterRecordReadPort, ClusterRecordView,
    CLUSTER_NUMBERS_PER_NODE, MAX_CLUSTER_RECORD_NAME_BYTES,
};

use super::cluster_settings::ClusterRefusal;
use super::persistence_slices::PersistableClusterRecord;
use super::store::RegistryStore;

const BLOCK_SIZE: usize = CLUSTER_NUMBERS_PER_NODE as usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClusterRecordEntry {
    pub index: u8,
    pub mark: ClusterBirthMark,
    pub name: FixedText64,
    pub revision: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClusterRecordBlock {
    pub entries: Vec<ClusterRecordEntry>,
    pub generations: [u32; BLOCK_SIZE],
}

impl ClusterRecordBlock {
    fn preallocated() -> Self {
        Self { entries: Vec::with_capacity(BLOCK_SIZE), generations: [0; BLOCK_SIZE] }
    }

    fn entry(&self, index: u8) -> Option<&ClusterRecordEntry> {
        self.entries.iter().find(|entry| entry.index == index)
    }

    fn live(&self) -> u16 {
        self.entries.iter().fold(0, |mask, entry| mask | 1 << entry.index)
    }

    fn generation(&self, index: u8) -> u32 {
        self.generations.get(usize::from(index)).copied().unwrap_or(0)
    }

    fn put(&mut self, entry: ClusterRecordEntry) {
        if let Some(slot) = self.generations.get_mut(usize::from(entry.index)) {
            *slot = (*slot).max(entry.mark.generation);
        }
        self.entries.retain(|held| held.index != entry.index);
        let at = self.entries.partition_point(|held| held.index < entry.index);
        self.entries.insert(at, entry);
    }

    fn remove(&mut self, index: u8) {
        self.entries.retain(|held| held.index != index);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClusterRecords {
    pub channels: ClusterRecordBlock,
    pub scenes: ClusterRecordBlock,
}

impl ClusterRecords {
    pub(crate) fn preallocated() -> Self {
        Self { channels: ClusterRecordBlock::preallocated(), scenes: ClusterRecordBlock::preallocated() }
    }

    pub(crate) const fn block(&self, kind: ClusterRecordKind) -> &ClusterRecordBlock {
        match kind {
            ClusterRecordKind::Channel => &self.channels,
            ClusterRecordKind::Scene => &self.scenes,
        }
    }

    const fn block_mut(&mut self, kind: ClusterRecordKind) -> &mut ClusterRecordBlock {
        match kind {
            ClusterRecordKind::Channel => &mut self.channels,
            ClusterRecordKind::Scene => &mut self.scenes,
        }
    }

    pub(crate) fn any_live(&self) -> bool {
        !self.channels.entries.is_empty() || !self.scenes.entries.is_empty()
    }

    pub(crate) fn apply(&mut self, edit: RecordEdit) {
        match edit {
            RecordEdit::Put { kind, entry } => self.block_mut(kind).put(entry),
            RecordEdit::Remove { kind, index } => self.block_mut(kind).remove(index),
        }
    }
}

pub(crate) enum RecordEdit {
    Put { kind: ClusterRecordKind, entry: ClusterRecordEntry },
    Remove { kind: ClusterRecordKind, index: u8 },
}

pub(crate) struct RecordRequest<'a> {
    pub command: &'a ClusterRecordUpdateCommand,
    pub name: Option<&'a FixedText64>,
}

pub(crate) fn plan_record_edit(
    records: &ClusterRecords,
    node: u8,
    request: &RecordRequest<'_>,
) -> Result<RecordEdit, ClusterRefusal> {
    let kind = request.command.kind;
    let block = records.block(kind);
    match request.command.action {
        ClusterRecordAction::Create { number, nonce } => {
            let entry = created(block, node, number, nonce, request)?;
            Ok(RecordEdit::Put { kind, entry })
        }
        ClusterRecordAction::Rename { number, revision } => {
            let held = held_at(block, node, number, revision)?;
            let name = request.name.ok_or(ClusterRefusal::OutOfRange)?.clone();
            let revision = held.revision.checked_add(1).ok_or(ClusterRefusal::OutOfRange)?;
            Ok(RecordEdit::Put { kind, entry: ClusterRecordEntry { name, revision, ..held.clone() } })
        }
        ClusterRecordAction::Delete { number, revision } => {
            let index = held_at(block, node, number, revision)?.index;
            Ok(RecordEdit::Remove { kind, index })
        }
    }
}

fn created(
    block: &ClusterRecordBlock,
    node: u8,
    number: Option<u8>,
    nonce: u32,
    request: &RecordRequest<'_>,
) -> Result<ClusterRecordEntry, ClusterRefusal> {
    let index = create_index(block, node, number)?;
    let generation = block.generation(index).checked_add(1).ok_or(ClusterRefusal::OutOfRange)?;
    let name = named(request.name, request.command.kind, cluster_block(node).start() + index);
    Ok(ClusterRecordEntry { index, mark: ClusterBirthMark { generation, nonce }, name, revision: 1 })
}

fn create_index(block: &ClusterRecordBlock, node: u8, number: Option<u8>) -> Result<u8, ClusterRefusal> {
    let conflict = |conflict| Err(ClusterRefusal::Record(conflict));
    let Some(number) = number else {
        let issued = |index| block.generation(index) > 0;
        let live = |index| block.entry(index).is_some();
        return next_record_index(issued, live).map_or_else(|| conflict(ClusterRecordConflict::BlockFull), Ok);
    };
    if !is_cluster_record_number(number) {
        return Err(ClusterRefusal::OutOfRange);
    }
    let Some(index) = block_index(node, number) else {
        return conflict(ClusterRecordConflict::NumberOfAnotherNode);
    };
    if block.entry(index).is_some() {
        return conflict(ClusterRecordConflict::NumberTaken);
    }
    Ok(index)
}

fn held_at(
    block: &ClusterRecordBlock,
    node: u8,
    number: u8,
    revision: u32,
) -> Result<&ClusterRecordEntry, ClusterRefusal> {
    let held = block_index(node, number).and_then(|index| block.entry(index)).ok_or(ClusterRefusal::RecordNotFound)?;
    if held.revision != revision {
        return Err(ClusterRefusal::Record(ClusterRecordConflict::StaleRevision));
    }
    Ok(held)
}

fn named(name: Option<&FixedText64>, kind: ClusterRecordKind, number: u8) -> FixedText64 {
    match name {
        Some(name) if !name.as_str().is_empty() => name.clone(),
        _ => dali2rust_contracts::msg::fixed_text_64(&default_record_name(kind, number)),
    }
}

fn record_view(kind: ClusterRecordKind, node: u8, entry: &ClusterRecordEntry) -> ClusterRecordView {
    ClusterRecordView {
        kind,
        number: cluster_block(node).start() + entry.index,
        home_node: node,
        mark: entry.mark,
        name: entry.name.as_str().to_string(),
        revision: entry.revision,
    }
}

impl ClusterRecordReadPort for RegistryStore {
    fn cluster_record(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView> {
        let g = self.read_inner();
        let node = g.cluster_settings.0.node_number;
        let index = block_index(node, number)?;
        g.cluster_records.block(kind).entry(index).map(|entry| record_view(kind, node, entry))
    }

    fn cluster_record_numbers(&self, kind: ClusterRecordKind) -> ClusterRecordNumbers {
        let g = self.read_inner();
        let first = *cluster_block(g.cluster_settings.0.node_number).start();
        ClusterRecordNumbers { first, live: g.cluster_records.block(kind).live() }
    }
}

pub(crate) fn persistable_records(block: &ClusterRecordBlock) -> Vec<PersistableClusterRecord> {
    block
        .entries
        .iter()
        .map(|entry| PersistableClusterRecord {
            index: entry.index,
            generation: entry.mark.generation,
            nonce: entry.mark.nonce,
            name: entry.name.as_str().to_string(),
            revision: entry.revision,
        })
        .collect()
}

pub(crate) fn stored_block(
    records: &[PersistableClusterRecord],
    generations: [u32; BLOCK_SIZE],
) -> Option<ClusterRecordBlock> {
    let mut block = ClusterRecordBlock { entries: Vec::with_capacity(BLOCK_SIZE), generations };
    for record in records {
        let issued = block.generations.get(usize::from(record.index)).copied()?;
        let fits = record.generation >= 1 && record.generation <= issued && record.revision >= 1;
        let named = record.name.len() <= MAX_CLUSTER_RECORD_NAME_BYTES;
        let later = block.entries.last().is_none_or(|last| last.index < record.index);
        if !(fits && named && later) {
            return None;
        }
        block.entries.push(ClusterRecordEntry {
            index: record.index,
            mark: ClusterBirthMark { generation: record.generation, nonce: record.nonce },
            name: dali2rust_contracts::msg::fixed_text_64(&record.name),
            revision: record.revision,
        });
    }
    Some(block)
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::{
        fixed_text_64, ClusterForwarding, ClusterSettingsField as Field, ClusterSettingsUpdateCommand,
        ClusterTextField, ClusterTextUpdateCommand,
    };
    use dali2rust_domain::registry::{ClusterSettingsReadPort, ClusterSettingsView};

    use super::super::cluster_settings::{
        decode_cluster_settings, hydrate_cluster_settings_inner, persistable_snapshot, ClusterPatch,
    };
    use super::super::persistence_slices::{
        encode_persistence_blob, PersistableClusterSettingsSlice, PersistenceEnvelope,
        CLUSTER_SETTINGS_SLICE_VERSION,
    };
    use super::super::store::Inner;
    use super::*;

    const SERIES: u64 = 11;
    const NONCE: u32 = 0x5eed;
    const CHANNEL: ClusterRecordKind = ClusterRecordKind::Channel;
    const SCENE: ClusterRecordKind = ClusterRecordKind::Scene;

    fn record(kind: ClusterRecordKind, action: ClusterRecordAction) -> ClusterPatch {
        ClusterPatch::Record(ClusterRecordUpdateCommand { series: SERIES, kind, action })
    }

    fn create(kind: ClusterRecordKind, number: Option<u8>) -> ClusterPatch {
        record(kind, ClusterRecordAction::Create { number, nonce: NONCE })
    }

    fn record_name(text: &str) -> ClusterPatch {
        ClusterPatch::Text(ClusterTextUpdateCommand {
            series: SERIES,
            field: ClusterTextField::RecordName,
            text: fixed_text_64(text),
        })
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
        body.patch(Field::NodeNumber);
        ClusterPatch::Settings(body)
    }

    fn commit(store: &RegistryStore, patches: Vec<ClusterPatch>) -> Result<(), ClusterRefusal> {
        let frames = u8::try_from(patches.len()).expect("frames");
        for patch in patches {
            store.stage_cluster_patch(SERIES, patch)?;
        }
        store.commit_cluster_settings(SERIES, frames).map(drop)
    }

    fn held(store: &RegistryStore, kind: ClusterRecordKind, number: u8) -> ClusterRecordView {
        store.cluster_record(kind, number).expect("record")
    }

    fn conflict(conflict: ClusterRecordConflict) -> Result<(), ClusterRefusal> {
        Err(ClusterRefusal::Record(conflict))
    }

    #[test]
    fn a_record_without_a_number_takes_the_lowest_number_of_the_node_s_block() {
        let store = RegistryStore::with_adapter_count(1);
        commit(&store, vec![node_number(2)]).expect("node 2");
        commit(&store, vec![create(CHANNEL, None), record_name("Corridor")]).expect("created");
        let made = held(&store, CHANNEL, 32);
        assert_eq!((made.home_node, made.name.as_str(), made.revision), (2, "Corridor", 1));
        assert_eq!(made.mark, ClusterBirthMark { generation: 1, nonce: NONCE });
        assert_eq!(store.cluster_record_numbers(CHANNEL), ClusterRecordNumbers { first: 32, live: 1 });
        assert_eq!(store.cluster_record_numbers(SCENE), ClusterRecordNumbers { first: 32, live: 0 });
    }

    #[test]
    fn a_record_without_a_name_is_named_after_its_kind_and_number() {
        let store = RegistryStore::with_adapter_count(1);
        commit(&store, vec![create(SCENE, Some(4))]).expect("created");
        commit(&store, vec![create(CHANNEL, Some(5)), record_name("")]).expect("created");
        assert_eq!(held(&store, SCENE, 4).name, "Scene 4");
        assert_eq!(held(&store, CHANNEL, 5).name, "Channel 5");
    }

    #[test]
    fn a_hand_set_number_is_issued_only_inside_the_block_and_only_once() {
        let store = RegistryStore::with_adapter_count(1);
        assert_eq!(commit(&store, vec![create(CHANNEL, Some(16))]), conflict(ClusterRecordConflict::NumberOfAnotherNode));
        assert_eq!(commit(&store, vec![create(CHANNEL, Some(128))]), Err(ClusterRefusal::OutOfRange));
        commit(&store, vec![create(CHANNEL, Some(15))]).expect("created");
        assert_eq!(commit(&store, vec![create(CHANNEL, Some(15))]), conflict(ClusterRecordConflict::NumberTaken));
        commit(&store, vec![create(SCENE, Some(15))]).expect("a scene has numbers of its own");
    }

    #[test]
    fn a_full_block_issues_no_more_and_a_freed_number_waits_for_the_never_issued_ones() {
        let store = RegistryStore::with_adapter_count(1);
        commit(&store, vec![create(CHANNEL, None)]).expect("0");
        commit(&store, vec![record(CHANNEL, ClusterRecordAction::Delete { number: 0, revision: 1 })]).expect("gone");
        for expected in 1..CLUSTER_NUMBERS_PER_NODE {
            commit(&store, vec![create(CHANNEL, None)]).expect("issued");
            assert!(store.cluster_record(CHANNEL, expected).is_some(), "number {expected}");
        }
        commit(&store, vec![create(CHANNEL, None)]).expect("the freed number");
        assert_eq!(held(&store, CHANNEL, 0).mark.generation, 2);
        assert_eq!(commit(&store, vec![create(CHANNEL, None)]), conflict(ClusterRecordConflict::BlockFull));
    }

    #[test]
    fn a_rename_and_a_delete_name_the_revision_they_saw() {
        let store = RegistryStore::with_adapter_count(1);
        commit(&store, vec![create(CHANNEL, Some(2)), record_name("Corridor")]).expect("created");
        let rename = |revision| record(CHANNEL, ClusterRecordAction::Rename { number: 2, revision });
        assert_eq!(commit(&store, vec![rename(0), record_name("Hall")]), conflict(ClusterRecordConflict::StaleRevision));
        commit(&store, vec![rename(1), record_name("Hall")]).expect("renamed");
        let renamed = held(&store, CHANNEL, 2);
        assert_eq!((renamed.name.as_str(), renamed.revision, renamed.mark.generation), ("Hall", 2, 1));
        let delete = |revision| record(CHANNEL, ClusterRecordAction::Delete { number: 2, revision });
        assert_eq!(commit(&store, vec![delete(1)]), conflict(ClusterRecordConflict::StaleRevision));
        commit(&store, vec![delete(2)]).expect("deleted");
        assert!(store.cluster_record(CHANNEL, 2).is_none());
        assert_eq!(commit(&store, vec![delete(2)]), Err(ClusterRefusal::RecordNotFound));
    }

    #[test]
    fn a_rename_without_a_name_and_a_name_without_a_record_are_refused() {
        let store = RegistryStore::with_adapter_count(1);
        commit(&store, vec![create(CHANNEL, Some(2)), record_name("Corridor")]).expect("created");
        let rename = record(CHANNEL, ClusterRecordAction::Rename { number: 2, revision: 1 });
        assert_eq!(commit(&store, vec![rename]), Err(ClusterRefusal::OutOfRange));
        assert_eq!(commit(&store, vec![record_name("Hall")]), Err(ClusterRefusal::OutOfRange));
        assert_eq!(commit(&store, vec![create(CHANNEL, None), create(SCENE, None)]), Err(ClusterRefusal::OutOfRange));
        assert_eq!(held(&store, CHANNEL, 2).name, "Corridor");
        assert_eq!(store.cluster_record_numbers(SCENE).live, 0);
    }

    #[test]
    fn the_node_number_is_pinned_only_when_it_moves_while_a_record_lives() {
        let store = RegistryStore::with_adapter_count(1);
        commit(&store, vec![node_number(1)]).expect("moves with no record");
        commit(&store, vec![create(SCENE, None)]).expect("created");
        commit(&store, vec![node_number(1)]).expect("names the number it has");
        assert_eq!(commit(&store, vec![node_number(2)]), conflict(ClusterRecordConflict::NodeNumberPinned));
        assert_eq!(store.cluster_settings_view().node_number, 1);
        commit(&store, vec![record(SCENE, ClusterRecordAction::Delete { number: 16, revision: 1 })]).expect("gone");
        commit(&store, vec![node_number(2)]).expect("moves once no record lives");
    }

    #[test]
    fn the_generation_of_an_index_is_kept_across_a_change_of_node_number() {
        let store = RegistryStore::with_adapter_count(1);
        commit(&store, vec![create(CHANNEL, Some(3))]).expect("created");
        commit(&store, vec![record(CHANNEL, ClusterRecordAction::Delete { number: 3, revision: 1 })]).expect("gone");
        commit(&store, vec![node_number(1)]).expect("moved");
        commit(&store, vec![create(CHANNEL, Some(19))]).expect("created");
        assert_eq!(held(&store, CHANNEL, 19).mark.generation, 2);
    }

    #[test]
    fn a_refused_record_leaves_the_settings_of_its_series_alone() {
        let store = RegistryStore::with_adapter_count(1);
        let refused = commit(&store, vec![node_number(4), create(CHANNEL, Some(3))]);
        assert_eq!(refused, conflict(ClusterRecordConflict::NumberOfAnotherNode));
        assert_eq!(store.cluster_settings_view(), ClusterSettingsView::default());
    }

    fn stored_with_records() -> PersistableClusterSettingsSlice {
        let store = RegistryStore::with_adapter_count(1);
        commit(&store, vec![create(CHANNEL, Some(1))]).expect("created");
        commit(&store, vec![record(CHANNEL, ClusterRecordAction::Delete { number: 1, revision: 1 })]).expect("gone");
        commit(&store, vec![create(CHANNEL, Some(1)), record_name("Hall")]).expect("again");
        commit(&store, vec![create(SCENE, Some(9)), record_name("Evening")]).expect("scene");
        let slice = persistable_snapshot(&store.read_inner());
        slice
    }

    fn decoded(slice: &PersistableClusterSettingsSlice) -> Option<PersistableClusterSettingsSlice> {
        let envelope = PersistenceEnvelope { version: CLUSTER_SETTINGS_SLICE_VERSION, data: slice.clone() };
        decode_cluster_settings(&encode_persistence_blob(&envelope).expect("encode")).ok()
    }

    #[test]
    fn records_and_generations_survive_the_stored_slice() {
        let slice = stored_with_records();
        let mut restored = Inner::default();
        hydrate_cluster_settings_inner(&mut restored, &decoded(&slice).expect("decodes"));
        let channel = restored.cluster_records.channels.entry(1).expect("channel");
        assert_eq!((channel.name.as_str(), channel.mark.generation, channel.mark.nonce), ("Hall", 2, NONCE));
        assert_eq!(restored.cluster_records.channels.generations[1], 2);
        assert_eq!(restored.cluster_records.scenes.entry(9).expect("scene").name.as_str(), "Evening");
        assert_eq!(restored.cluster_records.scenes.generations[9], 1);
    }

    type Break = fn(&mut PersistableClusterSettingsSlice);

    const BREAKS: [(&str, Break); 6] = [
        ("a generation never issued", |s| s.channels[0].generation = 3),
        ("generation zero", |s| s.channels[0].generation = 0),
        ("revision zero", |s| s.channels[0].revision = 0),
        ("an index past the block", |s| s.scenes[0].index = 16),
        ("an index twice", |s| s.channels.push(s.channels[0].clone())),
        ("a name too long", |s| s.scenes[0].name = "n".repeat(MAX_CLUSTER_RECORD_NAME_BYTES + 1)),
    ];

    #[test]
    fn a_stored_slice_whose_records_disagree_with_themselves_does_not_decode() {
        let slice = stored_with_records();
        assert!(decoded(&slice).is_some(), "the unbroken slice decodes");
        for (what, break_it) in BREAKS {
            let mut broken = slice.clone();
            break_it(&mut broken);
            assert!(decoded(&broken).is_none(), "{what}");
        }
    }
}
