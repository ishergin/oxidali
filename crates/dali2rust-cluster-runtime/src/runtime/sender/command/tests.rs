use dali2rust_contracts::msg::{LevelTransition, LightSetpoint, PowerState};

use crate::frame::{Action, LampAddress, LightVerb, Target};

use super::{action_refusal, CLUSTER_COMMAND_UNSUPPORTED};

const OWN_NODE: u8 = 2;

fn to_lamp(node: u8, verb: LightVerb) -> Action {
    Action::Light { target: Target::Lamp(LampAddress { node, adapter: 0, lamp: 1 }), verb }
}

fn level(value: u8) -> LightVerb {
    LightVerb::Setpoint(LightSetpoint { power: PowerState::On, level: Some(value), color: None })
}

#[test]
fn a_lamp_command_naming_this_node_is_refused_because_its_own_lamp_goes_down_the_virtual_lamp_s_route() {
    assert_eq!(action_refusal(&to_lamp(OWN_NODE, level(200)), OWN_NODE), Some(CLUSTER_COMMAND_UNSUPPORTED));
    assert_eq!(action_refusal(&to_lamp(OWN_NODE + 1, level(200)), OWN_NODE), None);
}

#[test]
fn a_lamp_command_leaves_only_with_a_verb_the_owner_executes() {
    let recall = LightVerb::Transition(LevelTransition::RecallMinLevel);
    assert_eq!(action_refusal(&to_lamp(OWN_NODE + 1, recall), OWN_NODE), None);
    let refused = [LightVerb::StopFade, LightVerb::Transition(LevelTransition::GoToLastActiveLevel)];
    for verb in refused {
        assert_eq!(action_refusal(&to_lamp(OWN_NODE + 1, verb), OWN_NODE), Some(CLUSTER_COMMAND_UNSUPPORTED));
    }
}
