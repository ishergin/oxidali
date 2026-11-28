use core::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;

use dali2rust_bus::{BusFrame, BusSubscriberRx};
use dali2rust_domain::registry::{ClusterPartReadPort, ClusterSettingsReadPort};
use dali2rust_platform::liveness::monotonic_ms;

use super::counters::{bump, ClusterCounters};
use super::execute::{ClusterExecution, Gate, Stamp};
use super::forwarding::Fact;
use super::hold::{Carrier, ForeignHold};
use super::shared::{CommandAdmission, CommandStanding, Heard, HeardCommand};
use crate::frame::{Action, Command, Target, WireNote};

pub(crate) struct CommandIntake {
    pub(crate) execution: Arc<ClusterExecution>,
    pub(crate) settings: Arc<dyn ClusterSettingsReadPort>,
    pub(crate) parts: Arc<dyn ClusterPartReadPort>,
    pub(crate) confirmations: BusSubscriberRx,
    pub(crate) hold: Arc<ForeignHold>,
}

impl CommandIntake {
    pub(crate) fn take(&self, heard: HeardCommand, counters: &ClusterCounters) {
        match heard.heard {
            Heard::Command(command) => self.take_command(&command, heard.admission, counters),
            Heard::Note(note) => self.take_note(&note, heard.admission, counters),
        }
    }

    fn take_note(&self, note: &WireNote, admission: CommandAdmission, counters: &ClusterCounters) {
        let CommandAdmission::Taken(standing) = admission else {
            return;
        };
        if !standing.running || note.wire != standing.wire {
            return;
        }
        bump(&counters.wire_notes_taken_total);
        let settings = self.settings.cluster_settings_view();
        if let Some(action) = Fact::noted(note, settings.send_all_lamps, &*self.parts) {
            self.cancel_held(&action, Carrier::Note, counters);
        }
    }

    fn take_command(&self, command: &Command, admission: CommandAdmission, counters: &ClusterCounters) {
        bump(&counters.commands_received_total);
        let Some(standing) = running_standing(admission, counters) else {
            return;
        };
        if let Action::Light { target: Target::Lamp(lamp), verb } = &command.action {
            return self.execution.execute_lamp(*lamp, verb, Stamp::of(command), standing.node).count_drop(counters);
        }
        if !self.wire_drops(command, standing, counters) {
            self.execution.execute(&command.action, Stamp::of(command), Gate::Accepting).count_drop(counters);
        }
    }

    fn wire_drops(&self, command: &Command, standing: CommandStanding, counters: &ClusterCounters) -> bool {
        if !self.settings.cluster_settings_view().wire_protection {
            return false;
        }
        if command.executed_on_wire == Some(standing.wire) {
            self.cancel_held(&command.action, Carrier::Command, counters);
            bump(&counters.commands_dropped_executed_on_wire_total);
            return true;
        }
        let foreign_wire = !standing.owns_wire;
        if foreign_wire {
            bump(&counters.commands_dropped_not_wire_owner_total);
        }
        foreign_wire
    }

    fn cancel_held(&self, action: &Action, carrier: Carrier, counters: &ClusterCounters) {
        let (cancelled, evicted) = self.hold.line().cancel(action, carrier, monotonic_ms());
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

fn running_standing(admission: CommandAdmission, counters: &ClusterCounters) -> Option<CommandStanding> {
    let standing = match admission {
        CommandAdmission::NotMember => {
            bump(&counters.commands_dropped_not_member_total);
            return None;
        }
        CommandAdmission::Duplicate => {
            bump(&counters.commands_dropped_duplicate_total);
            return None;
        }
        CommandAdmission::Taken(standing) => standing,
    };
    if !standing.running {
        bump(&counters.commands_dropped_not_running_total);
        return None;
    }
    Some(standing)
}
