use dali2rust_contracts::msg::{fixed_text_64, ClusterCopyAction, ClusterRecordCopyCommand, ClusterRecordKind, ClusterRecordRef};
use dali2rust_domain::registry::{cluster_node_of, ClusterCopyReadPort, ClusterPartReadPort, CLUSTER_RECORD_KINDS};

use crate::presence::{elapsed_ms, PresenceTiming};

use super::shared::BlockState;
use super::standing::{HomeReads, RecordStanding};

#[cfg(test)]
mod tests;

pub(crate) const COPY_COMMANDS_PER_PASS: usize = 16;

const ANSWER_PULSES: u32 = 2;

#[derive(Clone, Copy)]
pub(crate) struct CopyReads<'a> {
    pub(crate) home: HomeReads<'a>,
    pub(crate) parts: &'a dyn ClusterPartReadPort,
    pub(crate) copies: &'a dyn ClusterCopyReadPort,
    pub(crate) announced: Announced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Announced {
    pub(crate) sets_sent: [Option<u32>; 2],
    pub(crate) now_ms: u32,
    pub(crate) timing: PresenceTiming,
}

impl Announced {
    pub(crate) fn of(sets_sent: [Option<u32>; 2], own_block: BlockState, now_ms: u32, timing: PresenceTiming) -> Self {
        let sets_sent = if own_block == BlockState::Contested { [None; 2] } else { sets_sent };
        Self { sets_sent, now_ms, timing }
    }

    fn reached(&self, home: HomeReads<'_>, node: u8) -> bool {
        let (Some(view), Some(held)) = (home.presence.heard_node(node), home.info.info_held(node)) else {
            return false;
        };
        let Some(holding_ms) = view.settled_for_ms.map(|settled_ms| settled_ms.saturating_add(self.timing.settled_after_ms())) else {
            return false;
        };
        let [Some(earlier_ms), Some(last_ms)] = self.sets_sent else {
            return false;
        };
        let answer_ms = self.timing.info_coalesce_ms().saturating_add(ANSWER_PULSES.saturating_mul(self.timing.pulse_period_ms()));
        let both_since_heard = elapsed_ms(self.now_ms, earlier_ms) < holding_ms;
        both_since_heard && held.age_ms.saturating_add(answer_ms) <= elapsed_ms(self.now_ms, last_ms)
    }
}

pub(crate) fn copy_commands(own: u8, reads: CopyReads<'_>) -> Vec<ClusterRecordCopyCommand> {
    let mut commands = Vec::new();
    let mut seen: Vec<(ClusterRecordKind, ClusterRecordRef)> = Vec::new();
    for kind in CLUSTER_RECORD_KINDS {
        for (adapter, local) in reads.parts.cluster_part_keys(kind).iter() {
            let Some(part) = reads.parts.cluster_part(kind, adapter, local) else {
                continue;
            };
            let key = (kind, part.record);
            if cluster_node_of(part.record.number) == own || seen.contains(&key) {
                continue;
            }
            seen.push(key);
            commands.extend(copy_command(kind, part.record, reads));
            if commands.len() >= COPY_COMMANDS_PER_PASS {
                return commands;
            }
        }
    }
    commands
}

fn copy_command(kind: ClusterRecordKind, record: ClusterRecordRef, reads: CopyReads<'_>) -> Option<ClusterRecordCopyCommand> {
    let held = reads.copies.cluster_copy(kind, record);
    let home = reads.home.info.info_record(cluster_node_of(record.number), kind, record.number);
    let replaced = home.is_some();
    let action = match home.filter(|shown| shown.mark == record.mark) {
        Some(shown) if shown.revision >= 1 && held.as_ref().is_none_or(|copy| copy.revision < shown.revision) => {
            ClusterCopyAction::Store { revision: shown.revision, name: fixed_text_64(&shown.name) }
        }
        Some(_) => return None,
        None if held.is_some() && orphaned(kind, record, replaced, reads) => ClusterCopyAction::Drop,
        None => return None,
    };
    Some(ClusterRecordCopyCommand { kind, record, action })
}

fn orphaned(kind: ClusterRecordKind, record: ClusterRecordRef, replaced: bool, reads: CopyReads<'_>) -> bool {
    let orphan = reads.home.standing(kind, record) == RecordStanding::Orphan;
    orphan && (replaced || reads.announced.reached(reads.home, cluster_node_of(record.number)))
}
