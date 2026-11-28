use core::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusSubscriberRx};
use dali2rust_domain::registry::ClusterSettingsReadPort;
use dali2rust_platform::liveness::monotonic_ms;

use super::counters::{bump, ClusterCounters};
use super::execute::{ClusterExecution, Gate, Stamp};
use super::hold::ForeignHold;
use super::shared::{CommandAdmission, HeardCommand};
use crate::frame::Action;

pub(crate) struct CommandIntake {
    pub(crate) execution: Arc<ClusterExecution>,
    pub(crate) settings: Arc<dyn ClusterSettingsReadPort>,
    pub(crate) confirmations: BusSubscriberRx,
    pub(crate) hold: Arc<ForeignHold>,
}

impl CommandIntake {
    pub(crate) fn take(&self, heard: HeardCommand, counters: &ClusterCounters) {
        bump(&counters.commands_received_total);
        let standing = match heard.admission {
            CommandAdmission::NotMember => return bump(&counters.commands_dropped_not_member_total),
            CommandAdmission::Duplicate => return bump(&counters.commands_dropped_duplicate_total),
            CommandAdmission::Taken(standing) => standing,
        };
        if !standing.running {
            return bump(&counters.commands_dropped_not_running_total);
        }
        let command = &heard.command;
        if self.settings.cluster_settings_view().wire_protection {
            if command.executed_on_wire == Some(standing.wire) {
                self.cancel_held(&command.action, counters);
                return bump(&counters.commands_dropped_executed_on_wire_total);
            }
            if !standing.owns_wire {
                return bump(&counters.commands_dropped_not_wire_owner_total);
            }
        }
        self.execution.execute(&command.action, Stamp::of(command), Gate::Accepting).count_drop(counters);
    }

    fn cancel_held(&self, action: &Action, counters: &ClusterCounters) {
        let (cancelled, evicted) = self.hold.line().cancel(action, monotonic_ms());
        counters.foreign_cancelled_total.fetch_add(cancelled, Relaxed);
        if evicted {
            bump(&counters.foreign_hold_full_total);
        }
    }

    pub(crate) fn drain_confirmations(&self) {
        while let Ok(frame) = self.confirmations.try_recv() {
            if let BusFrame::Confirmation(confirmation) = frame {
                self.execution.note_confirmation(&confirmation);
            }
        }
    }
}
