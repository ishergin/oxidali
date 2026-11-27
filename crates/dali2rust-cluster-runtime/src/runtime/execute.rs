use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

use dali2rust_bus::{publish_or_drop, BusChannel, BusFrame, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    BusCommandPayload, ClusterRecordKind, ConfirmationEnvelope, DaliRecallLevelPresetCommand, DaliRecallSceneCommand,
    DaliSetTargetStateCommand, DeliveryStatus, LevelPreset, LevelTransition, LightSetpoint, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{AdapterReadPort, ClusterPartReadPort, ClusterPartView, ClusterSettingsReadPort};

use crate::frame::{Action, Command, LightVerb, RecallScope, RecordRef, SourceClass, Target};

use super::counters::{bump, ClusterCounters};

#[cfg(test)]
mod tests;

const CORRELATION_SHIFT: u32 = 32;
const CORRELATION_TAG: u64 = 0x434C_5553 << CORRELATION_SHIFT;
const CORRELATION_TAG_MASK: u64 = 0xFFFF_FFFF << CORRELATION_SHIFT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gate {
    Own,
    Accepting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) hold_hcl: bool,
    pub(crate) origin: Origin,
}

impl Stamp {
    pub(crate) const fn of(command: &Command) -> Self {
        Self { hold_hcl: command.hold_hcl, origin: origin_of(command.source) }
    }
}

pub(crate) const fn origin_of(source: SourceClass) -> Origin {
    match source {
        SourceClass::Hcl => Origin::ClusterHcl,
        SourceClass::Operator | SourceClass::HomeAssistant | SourceClass::Rules | SourceClass::ForeignMaster => {
            Origin::Cluster
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Pass {
    gate: Gate,
    publish: bool,
    stamp: Stamp,
}

impl Pass {
    const fn hold_hcl(self) -> bool {
        self.stamp.hold_hcl
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) unsupported: bool,
    pub(crate) matched: u8,
    pub(crate) accepted: u8,
    pub(crate) queued: u8,
}

impl Outcome {
    const UNSUPPORTED: Self = Self { unsupported: true, matched: 0, accepted: 0, queued: 0 };

    fn take(&mut self, accepts: bool, pass: Pass, publish: impl FnOnce() -> bool) {
        self.matched = self.matched.saturating_add(1);
        if pass.gate == Gate::Accepting && !accepts {
            return;
        }
        self.accepted = self.accepted.saturating_add(1);
        if pass.publish && publish() {
            self.queued = self.queued.saturating_add(1);
        }
    }

    pub(crate) fn count_drop(self, counters: &ClusterCounters) {
        let reason = if self.unsupported {
            &counters.commands_dropped_unsupported_total
        } else if self.accepted > 0 {
            return;
        } else if self.matched > 0 {
            &counters.commands_dropped_accept_off_total
        } else {
            &counters.commands_dropped_no_part_total
        };
        bump(reason);
    }
}

enum Verb {
    Setpoint(LightSetpoint),
    Preset(LevelPreset),
}

impl Verb {
    fn of(verb: &LightVerb) -> Option<Self> {
        match verb {
            LightVerb::Setpoint(setpoint) => Some(Self::Setpoint(setpoint.clone())),
            LightVerb::Transition(LevelTransition::RecallMaxLevel) => Some(Self::Preset(LevelPreset::MaxLevel)),
            LightVerb::Transition(LevelTransition::RecallMinLevel) => Some(Self::Preset(LevelPreset::MinLevel)),
            LightVerb::Transition(_) | LightVerb::StopFade => None,
        }
    }

    fn on_group(&self, part: &ClusterPartView, hold_hcl: bool) -> BusCommandPayload {
        match self {
            Self::Setpoint(setpoint) => {
                let command = DaliSetTargetStateCommand::for_group(part.adapter, part.local, setpoint);
                DaliSetTargetStateCommand { hold_hcl, ..command }.into()
            }
            Self::Preset(preset) => {
                let command = DaliRecallLevelPresetCommand::for_group(part.adapter, part.local, *preset);
                DaliRecallLevelPresetCommand { hold_hcl, ..command }.into()
            }
        }
    }

    fn on_all(&self, adapter: u8, hold_hcl: bool) -> BusCommandPayload {
        match self {
            Self::Setpoint(setpoint) => {
                DaliSetTargetStateCommand { hold_hcl, ..DaliSetTargetStateCommand::broadcast(adapter, setpoint) }.into()
            }
            Self::Preset(preset) => {
                DaliRecallLevelPresetCommand { hold_hcl, ..DaliRecallLevelPresetCommand::broadcast(adapter, *preset) }.into()
            }
        }
    }
}

pub(crate) fn is_supported(action: &Action) -> bool {
    match action {
        Action::Light { target: Target::Channel(_) | Target::All, verb } => Verb::of(verb).is_some(),
        Action::Light { target: Target::Lamp(_), .. } => false,
        Action::RecallScene { .. } => true,
    }
}

pub(crate) struct ExecutionReads {
    pub(crate) parts: Arc<dyn ClusterPartReadPort>,
    pub(crate) settings: Arc<dyn ClusterSettingsReadPort>,
    pub(crate) adapters: Arc<dyn AdapterReadPort>,
}

pub struct ClusterExecution {
    publisher: BusPublisher,
    bus_id: BusId,
    reads: ExecutionReads,
    counters: Arc<ClusterCounters>,
    next: AtomicU32,
}

const PLAN_STAMP: Stamp = Stamp { hold_hcl: false, origin: Origin::Cluster };

impl ClusterExecution {
    pub(crate) fn new(
        (publisher, bus_id): (BusPublisher, BusId),
        reads: ExecutionReads,
        counters: Arc<ClusterCounters>,
    ) -> Self {
        Self { publisher, bus_id, reads, counters, next: AtomicU32::new(0) }
    }

    pub(crate) fn execute(&self, action: &Action, stamp: Stamp, gate: Gate) -> Outcome {
        self.run(action, Pass { gate, publish: true, stamp })
    }

    pub(crate) fn plan(&self, action: &Action, gate: Gate) -> Outcome {
        self.run(action, Pass { gate, publish: false, stamp: PLAN_STAMP })
    }

    fn run(&self, action: &Action, pass: Pass) -> Outcome {
        match action {
            Action::Light { target, verb } => match (target, Verb::of(verb)) {
                (Target::Channel(channel), Some(verb)) => {
                    self.on_channel(*channel, pass, |part| verb.on_group(part, pass.hold_hcl()))
                }
                (Target::All, Some(verb)) => self.on_all(&verb, pass),
                (Target::Lamp(_), _) | (_, None) => Outcome::UNSUPPORTED,
            },
            Action::RecallScene { scene, scope: RecallScope::Node } => self.on_node(*scene, pass),
            Action::RecallScene { scene, scope: RecallScope::Channel(channel) } => {
                self.on_channel_scene(*scene, *channel, pass)
            }
        }
    }

    fn on_all(&self, verb: &Verb, pass: Pass) -> Outcome {
        let accepts = self.reads.settings.cluster_settings_view().accept_all_lamps;
        let mut outcome = Outcome::default();
        for adapter in 0..self.reads.adapters.adapter_count() {
            outcome.take(accepts, pass, || self.publish(verb.on_all(adapter, pass.hold_hcl()), pass.stamp.origin));
        }
        outcome
    }

    fn linked(&self, kind: ClusterRecordKind, record: RecordRef) -> impl Iterator<Item = ClusterPartView> + '_ {
        let parts = &*self.reads.parts;
        let keys = parts.cluster_part_keys(kind).iter();
        keys.filter_map(move |(adapter, local)| parts.cluster_part(kind, adapter, local))
            .filter(move |part| part.record == record)
    }

    fn on_channel(&self, channel: RecordRef, pass: Pass, build: impl Fn(&ClusterPartView) -> BusCommandPayload) -> Outcome {
        let mut outcome = Outcome::default();
        for part in self.linked(ClusterRecordKind::Channel, channel) {
            outcome.take(part.accept, pass, || self.publish(build(&part), pass.stamp.origin));
        }
        outcome
    }

    fn on_node(&self, scene: RecordRef, pass: Pass) -> Outcome {
        let mut outcome = Outcome::default();
        for part in self.linked(ClusterRecordKind::Scene, scene) {
            let recall = DaliRecallSceneCommand::broadcast(part.adapter, part.local);
            let recall = DaliRecallSceneCommand { hold_hcl: pass.hold_hcl(), ..recall };
            outcome.take(part.accept, pass, || self.publish(recall.into(), pass.stamp.origin));
        }
        outcome
    }

    fn on_channel_scene(&self, scene: RecordRef, channel: RecordRef, pass: Pass) -> Outcome {
        let mut outcome = Outcome::default();
        for scene_part in self.linked(ClusterRecordKind::Scene, scene) {
            let on_adapter = |part: &ClusterPartView| part.adapter == scene_part.adapter;
            let Some(group) = self.linked(ClusterRecordKind::Channel, channel).find(on_adapter) else {
                continue;
            };
            let command = DaliRecallSceneCommand::for_group(scene_part.adapter, group.local, scene_part.local);
            let recall = DaliRecallSceneCommand { hold_hcl: pass.hold_hcl(), ..command };
            outcome.take(scene_part.accept && group.accept, pass, || self.publish(recall.into(), pass.stamp.origin));
        }
        outcome
    }

    fn publish(&self, payload: BusCommandPayload, origin: Origin) -> bool {
        let correlation_id = CORRELATION_TAG | u64::from(self.next.fetch_add(1, Relaxed));
        let envelope = command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, self.bus_id.0, Some(origin), payload);
        let frame = BusFrame::command(envelope);
        let queued = publish_or_drop(&self.publisher, BusChannel::Commands, frame, "cluster-execute") == PublishResult::Queued;
        let counter = if queued { &self.counters.commands_executed_total } else { &self.counters.commands_rejected_total };
        bump(counter);
        queued
    }

    pub(crate) fn note_confirmation(&self, confirmation: &ConfirmationEnvelope) {
        let ours = confirmation.meta.correlation_id & CORRELATION_TAG_MASK == CORRELATION_TAG;
        if ours && confirmation.status != DeliveryStatus::Ok {
            bump(&self.counters.commands_rejected_total);
        }
    }
}
