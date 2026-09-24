use dali2rust_rules_model::{
    CrossDirection, GroupAggregate, InputEventMatch, InputSelector, OccupancyState,
    OnlineTransition, OverrideTransition, PowerTransition, Rule, Trigger, TriggerKind,
};

use super::input::{EngineInput, InputEventKind};
use super::state::{Volatile, INPUT_VALUE_MASK_10BIT};

pub(crate) fn rule_matches(
    rule: &Rule,
    input: &EngineInput<'_>,
    vol: &Volatile,
) -> Option<TriggerKind> {
    rule.triggers
        .iter()
        .find(|t| trigger_matches(t, input, vol))
        .map(Trigger::kind)
}

fn trigger_matches(trigger: &Trigger, input: &EngineInput<'_>, vol: &Volatile) -> bool {
    match input {
        EngineInput::InputEvent { .. } => input_event_matches(trigger, input, vol),
        EngineInput::LampChanged { .. } => lamp_matches(trigger, input),
        EngineInput::GroupChanged { .. } => group_matches(trigger, input),
        EngineInput::SceneRecalled { adapter_id, scene_id } => matches!(
            trigger,
            Trigger::SceneRecalled { adapter_id: a, scene }
                if a == adapter_id && scene == scene_id
        ),
        EngineInput::DevicePresence { .. } => presence_matches(trigger, input),
        EngineInput::HclOverride { started, target } => matches!(
            trigger,
            Trigger::HclOverride { target: t, transition }
                if t == target && *started == (*transition == OverrideTransition::Starts)
        ),
        EngineInput::ControllerActive { active } => {
            *active && matches!(trigger, Trigger::ControllerBecomesActive)
        }
        EngineInput::ControllerStarts => matches!(trigger, Trigger::ControllerStarts),
        EngineInput::PowerCycled { .. } => lifecycle_matches(trigger, input),
        EngineInput::ManualConfigChanged { .. } => lifecycle_matches(trigger, input),
        EngineInput::RuleFailed { name } => {
            matches!(trigger, Trigger::RuleFails { rule } if rule == name)
        }
        EngineInput::RunRule { .. } | EngineInput::Tick => false,
    }
}

fn lifecycle_matches(trigger: &Trigger, input: &EngineInput<'_>) -> bool {
    match (input, trigger) {
        (
            EngineInput::PowerCycled { adapter_id, short_address },
            Trigger::InputDevicePowerCycled { device },
        ) => device.adapter_id == *adapter_id && Some(device.device_short_address) == *short_address,
        (
            EngineInput::ManualConfigChanged { adapter_id, short_address },
            Trigger::InputDeviceManualConfigChanged { device },
        ) => device.adapter_id == *adapter_id && device.device_short_address == *short_address,
        _ => false,
    }
}

fn input_event_matches(trigger: &Trigger, input: &EngineInput<'_>, vol: &Volatile) -> bool {
    let EngineInput::InputEvent {
        adapter_id,
        short_address,
        instance_number,
        instance_type,
        instance_groups,
        kind,
        value,
    } = input
    else {
        return false;
    };
    match trigger {
        Trigger::InputEvent { source, event } => {
            source_matches(source, *adapter_id, *short_address, *instance_number, *instance_type, instance_groups)
                && event_matches(event, *kind, *value)
        }
        Trigger::InputOccupancy { source, becomes } => {
            instance_is(source, *adapter_id, *short_address, *instance_number)
                && occupancy_matches(*becomes, *kind)
        }
        Trigger::InputLightCross {
            source,
            direction,
            threshold,
        } => {
            instance_is(source, *adapter_id, *short_address, *instance_number)
                && light_value_kind(*kind)
                && light_crosses(vol, (source.adapter_id, source.device_short_address, source.instance_number), *direction, *threshold, *value)
        }
        Trigger::InputPositionChange { source } => {
            instance_is(source, *adapter_id, *short_address, *instance_number)
                && *kind == InputEventKind::PositionChanged
        }
        _ => false,
    }
}

fn instance_is(
    source: &dali2rust_rules_model::InputRef,
    adapter_id: u8,
    short_address: Option<u8>,
    instance_number: Option<u8>,
) -> bool {
    source.adapter_id == adapter_id
        && Some(source.device_short_address) == short_address
        && Some(source.instance_number) == instance_number
}

fn source_matches(
    source: &InputSelector,
    adapter_id: u8,
    short_address: Option<u8>,
    instance_number: Option<u8>,
    instance_type: Option<u8>,
    instance_groups: &[Option<u8>; 3],
) -> bool {
    match source {
        InputSelector::Instance(input) => {
            instance_is(input, adapter_id, short_address, instance_number)
        }
        InputSelector::Group(sel) => {
            sel.adapter_id == adapter_id
                && instance_groups.contains(&Some(sel.instance_group))
                && sel.instance_type.is_none_or(|t| Some(t) == instance_type)
        }
    }
}

fn event_matches(matcher: &InputEventMatch, kind: InputEventKind, value: u16) -> bool {
    match matcher {
        InputEventMatch::Press => kind == InputEventKind::Press,
        InputEventMatch::Release => kind == InputEventKind::Release,
        InputEventMatch::ShortPress => kind == InputEventKind::ShortPress,
        InputEventMatch::DoublePress => kind == InputEventKind::DoublePress,
        InputEventMatch::LongPressStart => kind == InputEventKind::LongPressStart,
        InputEventMatch::LongPressRepeat => kind == InputEventKind::LongPressRepeat,
        InputEventMatch::LongPressStop => kind == InputEventKind::LongPressStop,
        InputEventMatch::ButtonFree => kind == InputEventKind::ButtonFree,
        InputEventMatch::ButtonStuck => kind == InputEventKind::ButtonStuck,
        InputEventMatch::Movement => kind == InputEventKind::Movement,
        InputEventMatch::NoMovement => kind == InputEventKind::NoMovement,
        InputEventMatch::AnyEvent => true,
        InputEventMatch::Raw { data } => value == *data,
    }
}

fn occupancy_matches(becomes: OccupancyState, kind: InputEventKind) -> bool {
    match becomes {
        OccupancyState::Occupied => kind == InputEventKind::BecameOccupied,
        OccupancyState::Vacant => kind == InputEventKind::BecameVacant,
    }
}

fn light_value_kind(kind: InputEventKind) -> bool {
    matches!(
        kind,
        InputEventKind::LightCrossedAbove | InputEventKind::LightCrossedBelow
    )
}

fn light_crosses(
    vol: &Volatile,
    source: (u8, u8, u8),
    direction: CrossDirection,
    threshold: u16,
    value: u16,
) -> bool {
    if value == INPUT_VALUE_MASK_10BIT {
        return false;
    }
    let Some(prev) = vol.light_edge.get(&source).copied() else {
        return false;
    };
    match direction {
        CrossDirection::Above => prev <= threshold && value > threshold,
        CrossDirection::Below => prev >= threshold && value < threshold,
    }
}

fn lamp_matches(trigger: &Trigger, input: &EngineInput<'_>) -> bool {
    let EngineInput::LampChanged {
        adapter_id,
        lamp_id,
        is_on,
        level,
        was_on,
        previous_level,
    } = input
    else {
        return false;
    };
    match trigger {
        Trigger::LampTurns { lamp, to } => {
            lamp.adapter_id == *adapter_id
                && lamp.id == *lamp_id
                && was_on != is_on
                && (*to == PowerTransition::On) == *is_on
        }
        Trigger::LampLevelCross {
            lamp,
            direction,
            threshold,
        } => {
            lamp.adapter_id == *adapter_id
                && lamp.id == *lamp_id
                && match direction {
                    CrossDirection::Above => previous_level <= threshold && level > threshold,
                    CrossDirection::Below => previous_level >= threshold && level < threshold,
                }
        }
        _ => false,
    }
}

fn group_matches(trigger: &Trigger, input: &EngineInput<'_>) -> bool {
    let EngineInput::GroupChanged {
        adapter_id,
        group_id,
        any_on,
        was_any_on,
    } = input
    else {
        return false;
    };
    let Trigger::GroupBecomes { group, becomes } = trigger else {
        return false;
    };
    group.adapter_id == *adapter_id
        && group.id == *group_id
        && match becomes {
            GroupAggregate::AnyOn => !was_any_on && *any_on,
            GroupAggregate::AllOff => *was_any_on && !any_on,
        }
}

fn presence_matches(trigger: &Trigger, input: &EngineInput<'_>) -> bool {
    let EngineInput::DevicePresence {
        adapter_id,
        short_address,
        online,
    } = input
    else {
        return false;
    };
    let Trigger::DeviceOnlineTransition { device, transition } = trigger else {
        return false;
    };
    device.adapter_id == *adapter_id
        && device.short_address == *short_address
        && (*transition == OnlineTransition::ComesOnline) == *online
}
