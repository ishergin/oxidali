use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusSubscriberRx};
use dali2rust_domain::registry::ClusterSettingsReadPort;

use super::counters::{bump, ClusterCounters};
use super::execute::{ClusterExecution, Gate, Stamp};
use super::shared::{CommandAdmission, HeardCommand};

pub(crate) struct CommandIntake {
    pub(crate) execution: Arc<ClusterExecution>,
    pub(crate) settings: Arc<dyn ClusterSettingsReadPort>,
    pub(crate) confirmations: BusSubscriberRx,
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
                return bump(&counters.commands_dropped_executed_on_wire_total);
            }
            if !standing.owns_wire {
                return bump(&counters.commands_dropped_not_wire_owner_total);
            }
        }
        self.execution.execute(&command.action, Stamp::of(command), Gate::Accepting).count_drop(counters);
    }

    pub(crate) fn drain_confirmations(&self) {
        while let Ok(frame) = self.confirmations.try_recv() {
            if let BusFrame::Confirmation(confirmation) = frame {
                self.execution.note_confirmation(&confirmation);
            }
        }
    }
}
