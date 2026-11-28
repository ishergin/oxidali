use dali2rust_contracts::msg::{
    BusEventPayload, ClusterCommandCommand, ClusterForwarding, ClusterRecordKind, DaliObservedFrameEvent,
    DaliTargetScope, LevelTransition, LightSetpoint, ObservedKind, PowerState, RuntimeSource,
};
use dali2rust_domain::registry::{ClusterPartReadPort, ClusterSettingsView};

use crate::frame::{
    Action, LightVerb, NoteDeed as Deed, NoteReach as Reach, RecallScope, RecordRef, SourceClass, Target, WireNote,
};

#[cfg(test)]
mod tests;

const FOREIGN_HOLDS_HCL: bool = true;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skip {
    ClusterOrigin,
    SourceOff,
    Wire,
    NotSent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ForeignStanding {
    pub(crate) forwards: bool,
    pub(crate) shared: bool,
}

impl Reach {
    fn of(scope: DaliTargetScope, group: Option<u8>) -> Option<Self> {
        match scope {
            DaliTargetScope::Group => group.map(Self::Group),
            DaliTargetScope::Broadcast => Some(Self::Broadcast),
            DaliTargetScope::VirtualLamp | DaliTargetScope::Short | DaliTargetScope::AddressRange => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fact {
    adapter: u8,
    reach: Reach,
    deed: Deed,
    source: RuntimeSource,
    hold_hcl: bool,
    kept: bool,
}

impl Fact {
    pub(crate) fn of(payload: &BusEventPayload) -> Option<Self> {
        let mut kept = false;
        let (adapter, reach, deed, source, hold_hcl) = match payload {
            BusEventPayload::DaliTargetStateAppliedEvent(applied) => {
                let setpoint = Deed::Light(LightVerb::Setpoint(applied.setpoint.clone()));
                let reach = Reach::of(applied.scope, applied.group_id)?;
                (applied.registry_adapter_id, reach, setpoint, applied.source, applied.hold_hcl)
            }
            BusEventPayload::DaliLevelTransitionAppliedEvent(applied) => {
                kept = !travels(applied.transition);
                let deed = Deed::Light(foreign_transition(applied.transition));
                let reach = Reach::of(applied.scope, applied.group_id)?;
                (applied.registry_adapter_id, reach, deed, applied.source, applied.hold_hcl)
            }
            BusEventPayload::DaliStopFadeAppliedEvent(applied) => {
                let reach = Reach::of(applied.scope, applied.group_id)?;
                (applied.registry_adapter_id, reach, Deed::Light(LightVerb::StopFade), applied.source, false)
            }
            BusEventPayload::DaliObservedFrameEvent(observed) => {
                let deed = Deed::Light(observed_verb(observed)?);
                let reach = Reach::of(observed.scope, observed.group_id)?;
                (observed.registry_adapter_id, reach, deed, RuntimeSource::Sniffer, FOREIGN_HOLDS_HCL)
            }
            BusEventPayload::DaliSceneRecalledEvent(recalled) if recalled.error.is_none() => {
                let reach = Reach::of(recalled.scope, Some(recalled.group_id))?;
                (recalled.registry_adapter_id, reach, Deed::Scene(recalled.scene_id), recalled.source, recalled.hold_hcl)
            }
            _ => return None,
        };
        Some(Self { adapter, reach, deed, source, hold_hcl, kept })
    }

    pub(crate) fn noted(note: &WireNote, send_all_lamps: bool, parts: &dyn ClusterPartReadPort) -> Option<Action> {
        let (adapter, reach, deed) = (note.adapter, note.reach, note.deed.clone());
        let fact = Self { adapter, reach, deed, source: RuntimeSource::Sniffer, hold_hcl: false, kept: false };
        fact.action(send_all_lamps, parts)
    }

    pub(crate) fn note(&self, wire: u8) -> WireNote {
        WireNote { wire, adapter: self.adapter, reach: self.reach, deed: self.deed.clone() }
    }

    pub(crate) const fn foreign(&self) -> bool {
        matches!(self.source, RuntimeSource::Sniffer)
    }

    fn action(&self, send_all_lamps: bool, parts: &dyn ClusterPartReadPort) -> Option<Action> {
        let sent = |kind, local| sent_record(parts, kind, self.adapter, local);
        match (&self.deed, self.reach) {
            (Deed::Light(verb), Reach::Group(group)) => {
                let channel = sent(ClusterRecordKind::Channel, group)?;
                Some(Action::Light { target: Target::Channel(channel), verb: verb.clone() })
            }
            (Deed::Light(verb), Reach::Broadcast) => send_all_lamps.then(|| Action::Light { target: Target::All, verb: verb.clone() }),
            (Deed::Scene(scene), Reach::Broadcast) => {
                let scene = sent(ClusterRecordKind::Scene, *scene)?;
                Some(Action::RecallScene { scene, scope: RecallScope::Node })
            }
            (Deed::Scene(scene), Reach::Group(group)) => {
                let scene = sent(ClusterRecordKind::Scene, *scene)?;
                let channel = sent(ClusterRecordKind::Channel, group)?;
                Some(Action::RecallScene { scene, scope: RecallScope::Channel(channel) })
            }
        }
    }
}

fn observed_verb(observed: &DaliObservedFrameEvent) -> Option<LightVerb> {
    match observed.observed_kind {
        ObservedKind::TargetStateObserved => observed.setpoint.clone().map(LightVerb::Setpoint),
        ObservedKind::LevelTransitionObserved | ObservedKind::DimmingObserved => {
            observed.level_transition.map(foreign_transition)
        }
        ObservedKind::FadeStopObserved => Some(LightVerb::StopFade),
        ObservedKind::SceneRecallObserved
        | ObservedKind::UnknownObserved
        | ObservedKind::SceneWriteObserved
        | ObservedKind::SceneRemovalObserved => None,
    }
}

const fn foreign_transition(transition: LevelTransition) -> LightVerb {
    match transition {
        LevelTransition::GoToLastActiveLevel => {
            LightVerb::Setpoint(LightSetpoint { power: PowerState::On, level: None, color: None })
        }
        other => LightVerb::Transition(other),
    }
}

const fn travels(transition: LevelTransition) -> bool {
    match transition {
        LevelTransition::RecallMaxLevel
        | LevelTransition::RecallMinLevel
        | LevelTransition::Up
        | LevelTransition::Down
        | LevelTransition::StepUp
        | LevelTransition::StepDown
        | LevelTransition::OnAndStepUp
        | LevelTransition::StepDownAndOff => true,
        LevelTransition::GoToLastActiveLevel => false,
    }
}

fn sent_record(parts: &dyn ClusterPartReadPort, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<RecordRef> {
    parts.cluster_part(kind, adapter, local).filter(|part| part.send).map(|part| part.record)
}

fn source_class(source: RuntimeSource, allowed: &ClusterForwarding) -> Result<SourceClass, Skip> {
    let (class, switched_on) = match source {
        RuntimeSource::Api => (SourceClass::Operator, allowed.operator),
        RuntimeSource::Mqtt => (SourceClass::HomeAssistant, allowed.home_assistant),
        RuntimeSource::Hcl => (SourceClass::Hcl, allowed.hcl),
        RuntimeSource::Sniffer => (SourceClass::ForeignMaster, allowed.foreign_master),
        RuntimeSource::Cluster | RuntimeSource::ClusterHcl => return Err(Skip::ClusterOrigin),
        RuntimeSource::Rules
        | RuntimeSource::Poller
        | RuntimeSource::Readback
        | RuntimeSource::AdapterProxy => return Err(Skip::SourceOff),
    };
    if switched_on {
        Ok(class)
    } else {
        Err(Skip::SourceOff)
    }
}

pub(crate) fn forwarding(
    fact: &Fact,
    settings: &ClusterSettingsView,
    parts: &dyn ClusterPartReadPort,
    wire: ForeignStanding,
) -> Result<ClusterCommandCommand, Skip> {
    let source = source_class(fact.source, &settings.forwarding)?;
    if source == SourceClass::ForeignMaster && !wire.forwards {
        return Err(Skip::Wire);
    }
    let action = (!fact.kept).then(|| fact.action(settings.send_all_lamps, parts)).flatten().ok_or(Skip::NotSent)?;
    Ok(ClusterCommandCommand { action, source, hold_hcl: fact.hold_hcl })
}
