use dali2rust_contracts::msg::{fixed_text_64, ClusterCopyAction, ClusterRecordCopyCommand, ClusterRecordKind, ClusterRecordRef};
use dali2rust_domain::registry::{cluster_node_of, ClusterCopyReadPort, ClusterPartReadPort, CLUSTER_RECORD_KINDS};

use super::standing::{HomeReads, RecordStanding};

#[cfg(test)]
mod tests;

pub(crate) const COPY_COMMANDS_PER_PASS: usize = 16;

#[derive(Clone, Copy)]
pub(crate) struct CopyReads<'a> {
    pub(crate) home: HomeReads<'a>,
    pub(crate) parts: &'a dyn ClusterPartReadPort,
    pub(crate) copies: &'a dyn ClusterCopyReadPort,
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
    let action = match home.filter(|shown| shown.mark == record.mark) {
        Some(shown) if held.as_ref().is_none_or(|copy| copy.revision < shown.revision) => {
            ClusterCopyAction::Store { revision: shown.revision, name: fixed_text_64(&shown.name) }
        }
        Some(_) => return None,
        None if held.is_some() && reads.home.standing(kind, record) == RecordStanding::Orphan => ClusterCopyAction::Drop,
        None => return None,
    };
    Some(ClusterRecordCopyCommand { kind, record, action })
}
