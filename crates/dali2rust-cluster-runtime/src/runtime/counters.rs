use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

use crate::frame::Rejection;
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
}

pub(crate) fn bump(counter: &AtomicU32) {
    counter.fetch_add(1, Relaxed);
}

impl ClusterCounters {
    pub(crate) fn count_rejection(&self, rejection: Rejection) {
        match rejection {
            Rejection::UnknownVersion(_) => bump(&self.unknown_version_frames_total),
            Rejection::NotCluster
            | Rejection::UnknownKind(_)
            | Rejection::TooLarge
            | Rejection::Malformed => bump(&self.rejected_frames_total),
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
