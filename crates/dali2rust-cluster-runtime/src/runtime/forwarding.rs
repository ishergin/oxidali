use dali2rust_contracts::msg::{
    BusEventPayload, ClusterCommandCommand, ClusterForwarding, ClusterRecordKind, DaliTargetScope, LevelTransition,
    RuntimeSource,
};
use dali2rust_domain::registry::{ClusterPartReadPort, ClusterSettingsView};

use crate::frame::{Action, LightVerb, RecallScope, RecordRef, SourceClass, Target};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skip {
    ClusterOrigin,
    SourceOff,
    NotSent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Group(u8),
    Broadcast,
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
enum Deed {
    Light(LightVerb),
    Scene(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fact {
    adapter: u8,
    reach: Reach,
    deed: Deed,
    source: RuntimeSource,
    hold_hcl: bool,
}

impl Fact {
    pub(crate) fn of(payload: &BusEventPayload) -> Option<Self> {
        let (adapter, reach, deed, source, hold_hcl) = match payload {
            BusEventPayload::DaliTargetStateAppliedEvent(applied) => {
                let setpoint = Deed::Light(LightVerb::Setpoint(applied.setpoint.clone()));
                let reach = Reach::of(applied.scope, applied.group_id)?;
                (applied.registry_adapter_id, reach, setpoint, applied.source, applied.hold_hcl)
            }
            BusEventPayload::DaliLevelTransitionAppliedEvent(applied) => {
                let deed = Deed::Light(forwarded_transition(applied.transition)?);
                let reach = Reach::of(applied.scope, applied.group_id)?;
                (applied.registry_adapter_id, reach, deed, applied.source, applied.hold_hcl)
            }
            BusEventPayload::DaliStopFadeAppliedEvent(applied) => {
                let reach = Reach::of(applied.scope, applied.group_id)?;
                (applied.registry_adapter_id, reach, Deed::Light(LightVerb::StopFade), applied.source, false)
            }
            BusEventPayload::DaliSceneRecalledEvent(recalled) if recalled.error.is_none() => {
                let reach = Reach::of(recalled.scope, Some(recalled.group_id))?;
                (recalled.registry_adapter_id, reach, Deed::Scene(recalled.scene_id), recalled.source, recalled.hold_hcl)
            }
            _ => return None,
        };
        Some(Self { adapter, reach, deed, source, hold_hcl })
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

fn forwarded_transition(transition: LevelTransition) -> Option<LightVerb> {
    let forwarded = match transition {
        LevelTransition::RecallMaxLevel
        | LevelTransition::RecallMinLevel
        | LevelTransition::Up
        | LevelTransition::Down
        | LevelTransition::StepUp
        | LevelTransition::StepDown
        | LevelTransition::OnAndStepUp
        | LevelTransition::StepDownAndOff => true,
        LevelTransition::GoToLastActiveLevel => false,
    };
    forwarded.then_some(LightVerb::Transition(transition))
}

fn sent_record(parts: &dyn ClusterPartReadPort, kind: ClusterRecordKind, adapter: u8, local: u8) -> Option<RecordRef> {
    parts.cluster_part(kind, adapter, local).filter(|part| part.send).map(|part| part.record)
}

fn source_class(source: RuntimeSource, allowed: &ClusterForwarding) -> Result<SourceClass, Skip> {
    let (class, switched_on) = match source {
        RuntimeSource::Api => (SourceClass::Operator, allowed.operator),
        RuntimeSource::Mqtt => (SourceClass::HomeAssistant, allowed.home_assistant),
        RuntimeSource::Hcl => (SourceClass::Hcl, allowed.hcl),
        RuntimeSource::Cluster | RuntimeSource::ClusterHcl => return Err(Skip::ClusterOrigin),
        RuntimeSource::Sniffer
        | RuntimeSource::Rules
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
) -> Result<ClusterCommandCommand, Skip> {
    let source = source_class(fact.source, &settings.forwarding)?;
    let action = fact.action(settings.send_all_lamps, parts).ok_or(Skip::NotSent)?;
    Ok(ClusterCommandCommand { action, source, hold_hcl: fact.hold_hcl })
}
