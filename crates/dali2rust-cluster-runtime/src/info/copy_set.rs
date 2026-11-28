use core::mem::MaybeUninit;
use core::ptr;

use dali2rust_bsp::psram::PsramBox;
use dali2rust_contracts::msg::ClusterRecordKind;
use dali2rust_domain::registry::{ClusterCopyReadPort, CLUSTER_RECORD_KINDS, MAX_CLUSTER_COPIES_PER_KIND};

use crate::frame::InfoElement;

use super::set::{held_record, held_to_record, write_empty, HeldRecord};

type HeldCopy = Option<(u8, HeldRecord)>;

pub(crate) struct CopySet {
    channels: [HeldCopy; MAX_CLUSTER_COPIES_PER_KIND],
    scenes: [HeldCopy; MAX_CLUSTER_COPIES_PER_KIND],
}

impl CopySet {
    const fn of_kind(&self, kind: ClusterRecordKind) -> &[HeldCopy; MAX_CLUSTER_COPIES_PER_KIND] {
        match kind {
            ClusterRecordKind::Channel => &self.channels,
            ClusterRecordKind::Scene => &self.scenes,
        }
    }

    fn of_kind_mut(&mut self, kind: ClusterRecordKind) -> &mut [HeldCopy; MAX_CLUSTER_COPIES_PER_KIND] {
        match kind {
            ClusterRecordKind::Channel => &mut self.channels,
            ClusterRecordKind::Scene => &mut self.scenes,
        }
    }

    pub(crate) fn fill(&mut self, copies: &dyn ClusterCopyReadPort) {
        for kind in CLUSTER_RECORD_KINDS {
            for (index, slot) in self.of_kind_mut(kind).iter_mut().enumerate() {
                *slot = copies.cluster_copy_at(kind, index).map(|view| (view.number, held_record(&view)));
            }
        }
    }

    pub(crate) fn elements(&self) -> impl Iterator<Item = InfoElement<'_>> + '_ {
        let held = |kind| self.of_kind(kind).iter().flatten().map(move |(number, held)| (kind, held_to_record(held, *number)));
        held(ClusterRecordKind::Channel).chain(held(ClusterRecordKind::Scene)).map(|(kind, record)| match kind {
            ClusterRecordKind::Channel => InfoElement::ChannelRecord(record),
            ClusterRecordKind::Scene => InfoElement::SceneRecord(record),
        })
    }
}

pub(crate) fn placed_copy_set() -> PsramBox<CopySet> {
    // SAFETY: both arrays of the place are written once with empty slots, and nothing is read.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<CopySet>| {
            let set = place.as_mut_ptr();
            write_empty(ptr::addr_of_mut!((*set).channels).cast::<HeldCopy>(), MAX_CLUSTER_COPIES_PER_KIND);
            write_empty(ptr::addr_of_mut!((*set).scenes).cast::<HeldCopy>(), MAX_CLUSTER_COPIES_PER_KIND);
        })
    }
}
