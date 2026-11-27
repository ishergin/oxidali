use core::ptr;

use dali2rust_contracts::msg::{fixed_text_64, ClusterRecordKind, FixedText64};
use dali2rust_domain::registry::{
    block_index, cluster_block, ClusterPartKeys, ClusterPartReadPort, ClusterPartView, ClusterRecordNumbers,
    ClusterRecordReadPort, ClusterRecordView, CLUSTER_PART_LOCALS, CLUSTER_RECORD_KINDS,
};

use crate::frame::{
    BirthMark, ChannelPart, InfoElement, PartFlags, Record, RecordRef, ScenePart, ADAPTER_COUNT, NUMBERS_PER_NODE,
};

pub const BLOCK_SLOTS: usize = NUMBERS_PER_NODE as usize;

pub const PART_SLOTS: usize = ADAPTER_COUNT as usize * CLUSTER_PART_LOCALS as usize;

const LOCALS: usize = CLUSTER_PART_LOCALS as usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldRecord {
    pub mark: BirthMark,
    pub name: FixedText64,
    pub revision: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeldPart {
    pub record: RecordRef,
    pub send: bool,
    pub accept: bool,
    pub revision: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Kept {
    pub skipped: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct InfoSet {
    pub(crate) node: u8,
    pub(crate) channels: [Option<HeldRecord>; BLOCK_SLOTS],
    pub(crate) scenes: [Option<HeldRecord>; BLOCK_SLOTS],
    pub(crate) channel_parts: [Option<HeldPart>; PART_SLOTS],
    pub(crate) scene_parts: [Option<HeldPart>; PART_SLOTS],
    pub(crate) skipped: u16,
}

impl InfoSet {
    pub(crate) unsafe fn init(set: *mut Self) {
        // SAFETY: the caller hands a place of `InfoSet`; every field is written once here and nothing is read.
        unsafe {
            ptr::addr_of_mut!((*set).node).write(0);
            write_empty(ptr::addr_of_mut!((*set).channels).cast::<Option<HeldRecord>>(), BLOCK_SLOTS);
            write_empty(ptr::addr_of_mut!((*set).scenes).cast::<Option<HeldRecord>>(), BLOCK_SLOTS);
            write_empty(ptr::addr_of_mut!((*set).channel_parts).cast::<Option<HeldPart>>(), PART_SLOTS);
            write_empty(ptr::addr_of_mut!((*set).scene_parts).cast::<Option<HeldPart>>(), PART_SLOTS);
            ptr::addr_of_mut!((*set).skipped).write(0);
        }
    }

    pub(crate) fn clear(&mut self, node: u8) {
        self.node = node;
        self.channels.iter_mut().chain(self.scenes.iter_mut()).for_each(|slot| *slot = None);
        self.channel_parts.iter_mut().chain(self.scene_parts.iter_mut()).for_each(|slot| *slot = None);
        self.skipped = 0;
    }

    pub(crate) const fn records(&self, kind: ClusterRecordKind) -> &[Option<HeldRecord>; BLOCK_SLOTS] {
        match kind {
            ClusterRecordKind::Channel => &self.channels,
            ClusterRecordKind::Scene => &self.scenes,
        }
    }

    pub(crate) const fn parts(&self, kind: ClusterRecordKind) -> &[Option<HeldPart>; PART_SLOTS] {
        match kind {
            ClusterRecordKind::Channel => &self.channel_parts,
            ClusterRecordKind::Scene => &self.scene_parts,
        }
    }

    fn records_mut(&mut self, kind: ClusterRecordKind) -> &mut [Option<HeldRecord>; BLOCK_SLOTS] {
        match kind {
            ClusterRecordKind::Channel => &mut self.channels,
            ClusterRecordKind::Scene => &mut self.scenes,
        }
    }

    fn parts_mut(&mut self, kind: ClusterRecordKind) -> &mut [Option<HeldPart>; PART_SLOTS] {
        match kind {
            ClusterRecordKind::Channel => &mut self.channel_parts,
            ClusterRecordKind::Scene => &mut self.scene_parts,
        }
    }

    pub(crate) fn take(&mut self, element: &InfoElement<'_>) -> Kept {
        let kept = match element {
            InfoElement::ChannelRecord(record) => self.take_record(ClusterRecordKind::Channel, record),
            InfoElement::SceneRecord(record) => self.take_record(ClusterRecordKind::Scene, record),
            InfoElement::ChannelPart(part) => self.take_channel_part(part),
            InfoElement::ScenePart(part) => self.take_scene_part(part),
            InfoElement::GroupMap(_) | InfoElement::GroupName(_) | InfoElement::SceneName(_) => {
                Kept { skipped: true }
            }
        };
        if kept.skipped {
            self.skipped = self.skipped.saturating_add(1);
        }
        kept
    }

    fn take_record(&mut self, kind: ClusterRecordKind, record: &Record<'_>) -> Kept {
        let Some(index) = block_index(self.node, record.id.number) else {
            return Kept { skipped: true };
        };
        let held = HeldRecord { mark: record.id.mark, name: fixed_text_64(record.name), revision: record.revision };
        put(self.records_mut(kind), usize::from(index), held);
        Kept::default()
    }

    fn take_channel_part(&mut self, part: &ChannelPart) -> Kept {
        let held = HeldPart {
            record: part.channel,
            send: part.flags.send,
            accept: part.flags.accept,
            revision: part.revision,
        };
        put(self.parts_mut(ClusterRecordKind::Channel), part_slot(part.adapter, part.group), held);
        Kept { skipped: part.taken || part.desired_lamps != 0 || part.written_lamps != 0 }
    }

    fn take_scene_part(&mut self, part: &ScenePart) -> Kept {
        let held = HeldPart {
            record: part.scene,
            send: part.flags.send,
            accept: part.flags.accept,
            revision: part.revision,
        };
        put(self.parts_mut(ClusterRecordKind::Scene), part_slot(part.adapter, part.node_scene), held);
        Kept::default()
    }

    pub(crate) fn fill(&mut self, node: u8, records: &dyn ClusterRecordReadPort, parts: &dyn ClusterPartReadPort) {
        self.clear(node);
        for kind in CLUSTER_RECORD_KINDS {
            for number in records.cluster_record_numbers(kind).iter() {
                let index = block_index(node, number).map(usize::from);
                if let (Some(index), Some(view)) = (index, records.cluster_record(kind, number)) {
                    put(self.records_mut(kind), index, held_record(&view));
                }
            }
            for (adapter, local) in parts.cluster_part_keys(kind).iter() {
                if let Some(view) = parts.cluster_part(kind, adapter, local) {
                    put(self.parts_mut(kind), part_slot(adapter, local), held_part(&view));
                }
            }
        }
    }

    pub fn elements(&self) -> impl Iterator<Item = InfoElement<'_>> + '_ {
        let first = *cluster_block(self.node).start();
        let records = |kind| {
            self.records(kind).iter().zip(first..).filter_map(move |(slot, number)| {
                slot.as_ref().map(|held| (kind, held_to_record(held, number)))
            })
        };
        let records = records(ClusterRecordKind::Channel).chain(records(ClusterRecordKind::Scene)).map(
            |(kind, record)| match kind {
                ClusterRecordKind::Channel => InfoElement::ChannelRecord(record),
                ClusterRecordKind::Scene => InfoElement::SceneRecord(record),
            },
        );
        records.chain(self.part_elements())
    }

    fn part_elements(&self) -> impl Iterator<Item = InfoElement<'_>> + '_ {
        let channel = self.part_keys(ClusterRecordKind::Channel).map(channel_part_element);
        channel.chain(self.part_keys(ClusterRecordKind::Scene).map(scene_part_element))
    }

    fn part_keys(&self, kind: ClusterRecordKind) -> impl Iterator<Item = (u8, u8, HeldPart)> + '_ {
        self.parts(kind).iter().enumerate().filter_map(|(slot, held)| {
            let (adapter, local) = slot_key(slot)?;
            held.map(|held| (adapter, local, held))
        })
    }

    pub(crate) fn record_view(&self, kind: ClusterRecordKind, number: u8) -> Option<ClusterRecordView> {
        let index = usize::from(block_index(self.node, number)?);
        let held = self.records(kind).get(index)?.as_ref()?;
        Some(ClusterRecordView {
            kind,
            number,
            home_node: self.node,
            mark: held.mark,
            name: held.name.as_str().to_owned(),
            revision: held.revision,
        })
    }

    pub(crate) fn record_numbers(&self, kind: ClusterRecordKind) -> ClusterRecordNumbers {
        let live = self.records(kind).iter().enumerate().fold(0_u16, |live, (index, slot)| {
            if slot.is_some() {
                live | 1_u16.checked_shl(u32::try_from(index).unwrap_or(u32::MAX)).unwrap_or(0)
            } else {
                live
            }
        });
        ClusterRecordNumbers { first: *cluster_block(self.node).start(), live }
    }

    pub(crate) fn part_view(&self, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<ClusterPartView> {
        if adapter >= ADAPTER_COUNT || local >= CLUSTER_PART_LOCALS {
            return None;
        }
        let held = (*self.parts(kind).get(part_slot(adapter, local))?)?;
        Some(ClusterPartView {
            kind,
            adapter,
            local,
            record: held.record,
            send: held.send,
            accept: held.accept,
            revision: held.revision,
        })
    }

    pub(crate) fn part_key_set(&self, kind: ClusterRecordKind) -> ClusterPartKeys {
        let mut keys = ClusterPartKeys::default();
        for (adapter, local, _) in self.part_keys(kind) {
            keys.insert(adapter, local);
        }
        keys
    }

    pub(crate) fn count(&self, kind: ClusterRecordKind) -> (u8, u8) {
        let records = self.records(kind).iter().flatten().count();
        let parts = self.parts(kind).iter().flatten().count();
        (u8::try_from(records).unwrap_or(u8::MAX), u8::try_from(parts).unwrap_or(u8::MAX))
    }
}

unsafe fn write_empty<T>(first: *mut Option<T>, slots: usize) {
    for index in 0..slots {
        // SAFETY: the caller hands the first of `slots` contiguous places of an array field.
        unsafe { first.add(index).write(None) };
    }
}

fn put<T>(slots: &mut [Option<T>], index: usize, value: T) {
    if let Some(slot) = slots.get_mut(index) {
        *slot = Some(value);
    }
}

const fn part_slot(adapter: u8, local: u8) -> usize {
    adapter as usize * LOCALS + local as usize
}

fn slot_key(slot: usize) -> Option<(u8, u8)> {
    let adapter = u8::try_from(slot / LOCALS).ok()?;
    let local = u8::try_from(slot % LOCALS).ok()?;
    Some((adapter, local))
}

fn held_record(view: &ClusterRecordView) -> HeldRecord {
    HeldRecord { mark: view.mark, name: fixed_text_64(&view.name), revision: view.revision }
}

const fn held_part(view: &ClusterPartView) -> HeldPart {
    HeldPart { record: view.record, send: view.send, accept: view.accept, revision: view.revision }
}

fn held_to_record(held: &HeldRecord, number: u8) -> Record<'_> {
    Record { id: RecordRef { number, mark: held.mark }, name: held.name.as_str(), revision: held.revision }
}

fn channel_part_element<'a>((adapter, group, held): (u8, u8, HeldPart)) -> InfoElement<'a> {
    InfoElement::ChannelPart(ChannelPart {
        adapter,
        channel: held.record,
        group,
        taken: false,
        flags: PartFlags { send: held.send, accept: held.accept },
        revision: held.revision,
        desired_lamps: 0,
        written_lamps: 0,
    })
}

fn scene_part_element<'a>((adapter, node_scene, held): (u8, u8, HeldPart)) -> InfoElement<'a> {
    let flags = PartFlags { send: held.send, accept: held.accept };
    InfoElement::ScenePart(ScenePart { adapter, scene: held.record, node_scene, flags, revision: held.revision })
}
