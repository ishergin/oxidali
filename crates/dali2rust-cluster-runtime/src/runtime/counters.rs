use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

use super::forwarding::Skip;
use crate::frame::Rejection;
use crate::info::PageTally;
use crate::presence::Taken;

#[derive(Debug, Default)]
pub struct ClusterCounters {
    pub datagrams_sent_total: AtomicU32,
    pub send_failures_total: AtomicU32,
    pub bind_failures_total: AtomicU32,
    pub datagrams_received_total: AtomicU32,
    pub receive_failures_total: AtomicU32,
    pub own_frames_dropped_total: AtomicU32,
    pub other_cluster_frames_total: AtomicU32,
    pub unknown_version_frames_total: AtomicU32,
    pub rejected_frames_total: AtomicU32,
    pub duplicates_total: AtomicU32,
    pub stale_link_frames_total: AtomicU32,
    pub sequence_gaps_total: AtomicU32,
    pub withheld_standby_total: AtomicU32,
    pub boards_evicted_total: AtomicU32,
    pub info_sets_sent_total: AtomicU32,
    pub info_sets_taken_total: AtomicU32,
    pub info_sets_abandoned_total: AtomicU32,
    pub info_pages_ignored_total: AtomicU32,
    pub info_pages_stale_total: AtomicU32,
    pub info_elements_skipped_total: AtomicU32,
    pub commands_sent_total: AtomicU32,
    pub commands_delayed_total: AtomicU32,
    pub commands_superseded_total: AtomicU32,
    pub relative_verbs_dropped_total: AtomicU32,
    pub commands_received_total: AtomicU32,
    pub commands_executed_total: AtomicU32,
    pub commands_rejected_total: AtomicU32,
    pub commands_dropped_not_member_total: AtomicU32,
    pub commands_dropped_duplicate_total: AtomicU32,
    pub commands_dropped_not_running_total: AtomicU32,
    pub commands_dropped_unsupported_total: AtomicU32,
    pub commands_dropped_no_part_total: AtomicU32,
    pub commands_dropped_accept_off_total: AtomicU32,
    pub commands_dropped_not_wire_owner_total: AtomicU32,
    pub commands_dropped_executed_on_wire_total: AtomicU32,
    pub commands_forwarded_total: AtomicU32,
    pub forwards_skipped_cluster_origin_total: AtomicU32,
    pub forwards_skipped_source_off_total: AtomicU32,
    pub forwards_skipped_not_sent_total: AtomicU32,
    pub forwards_skipped_wire_total: AtomicU32,
    pub foreign_forwarded_total: AtomicU32,
    pub foreign_held_total: AtomicU32,
    pub foreign_cancelled_total: AtomicU32,
    pub foreign_hold_full_total: AtomicU32,
}

pub(crate) fn bump(counter: &AtomicU32) {
    counter.fetch_add(1, Relaxed);
}

impl ClusterCounters {
    pub(crate) const fn skipped(&self, skip: Skip) -> &AtomicU32 {
        match skip {
            Skip::ClusterOrigin => &self.forwards_skipped_cluster_origin_total,
            Skip::SourceOff => &self.forwards_skipped_source_off_total,
            Skip::NotSent => &self.forwards_skipped_not_sent_total,
            Skip::Wire => &self.forwards_skipped_wire_total,
        }
    }

    pub(crate) fn count_rejection(&self, rejection: Rejection) {
        match rejection {
            Rejection::UnknownVersion(_) => bump(&self.unknown_version_frames_total),
            Rejection::NotCluster
            | Rejection::UnknownKind(_)
            | Rejection::TooLarge
            | Rejection::Malformed => bump(&self.rejected_frames_total),
        }
    }

    pub(crate) fn count_page(&self, tally: PageTally) {
        let flags = [
            (tally.stale, &self.info_pages_stale_total),
            (tally.abandoned, &self.info_sets_abandoned_total),
            (tally.taken, &self.info_sets_taken_total),
        ];
        for (_, counter) in flags.into_iter().filter(|(raised, _)| *raised) {
            bump(counter);
        }
        self.info_elements_skipped_total.fetch_add(u32::from(tally.skipped), Relaxed);
    }

    pub(crate) fn count_unfed_page(&self, member: bool, taken: Option<Taken>) {
        if taken.is_none() {
            return;
        }
        if !member {
            bump(&self.info_pages_ignored_total);
        } else if taken == Some(Taken::Late) {
            bump(&self.info_pages_stale_total);
        }
    }

    pub(crate) fn count_taken(&self, taken: Taken) {
        match taken {
            Taken::Repeated => bump(&self.duplicates_total),
            Taken::Fresh { skipped } => {
                self.sequence_gaps_total.fetch_add(skipped, Relaxed);
            }
            Taken::First { evicted: true } => bump(&self.boards_evicted_total),
            Taken::First { evicted: false } | Taken::Restarted | Taken::Late | Taken::Unknown => {}
        }
    }
}
