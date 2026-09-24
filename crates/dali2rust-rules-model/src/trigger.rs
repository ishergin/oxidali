use crate::refs::{DeviceRef, GroupRef, InputDeviceRef, InputRef, InputSelector, LampRef, LightTarget};
use crate::time::{DaySet, DurationMs, SolarEvent, TimeOfDay};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    InputEvent,
    InputOccupancy,
    InputLightCross,
    InputPositionChange,
    InputDevicePowerCycled,
    InputDeviceManualConfigChanged,
    AtTime,
    AtSolar,
    Every,
    LampTurns,
    LampLevelCross,
    GroupBecomes,
    SceneRecalled,
    DeviceOnlineTransition,
    HclOverride,
    TimerFires,
    ControllerStarts,
    ControllerBecomesActive,
    RuleFails,
    HttpTrigger,
}

pub const TRIGGER_KIND_COUNT: usize = 20;

const TRIGGER_KIND_NAMES: [&str; TRIGGER_KIND_COUNT] = [
    "input_event",
    "input_occupancy",
    "input_light_cross",
    "input_position_change",
    "input_device_power_cycled",
    "input_device_manual_config_changed",
    "at_time",
    "at_solar",
    "every",
    "lamp_turns",
    "lamp_level_cross",
    "group_becomes",
    "scene_recalled",
    "device_online_transition",
    "hcl_override",
    "timer_fires",
    "controller_starts",
    "controller_becomes_active",
    "rule_fails",
    "http_trigger",
];

impl TriggerKind {
    pub const ALL: [TriggerKind; TRIGGER_KIND_COUNT] = [
        TriggerKind::InputEvent,
        TriggerKind::InputOccupancy,
        TriggerKind::InputLightCross,
        TriggerKind::InputPositionChange,
        TriggerKind::InputDevicePowerCycled,
        TriggerKind::InputDeviceManualConfigChanged,
        TriggerKind::AtTime,
        TriggerKind::AtSolar,
        TriggerKind::Every,
        TriggerKind::LampTurns,
        TriggerKind::LampLevelCross,
        TriggerKind::GroupBecomes,
        TriggerKind::SceneRecalled,
        TriggerKind::DeviceOnlineTransition,
        TriggerKind::HclOverride,
        TriggerKind::TimerFires,
        TriggerKind::ControllerStarts,
        TriggerKind::ControllerBecomesActive,
        TriggerKind::RuleFails,
        TriggerKind::HttpTrigger,
    ];

    pub fn name(self) -> &'static str {
        TRIGGER_KIND_NAMES[self as usize]
    }

    pub fn is_input_class(self) -> bool {
        matches!(
            self,
            TriggerKind::InputEvent
                | TriggerKind::InputOccupancy
                | TriggerKind::InputLightCross
                | TriggerKind::InputPositionChange
                | TriggerKind::InputDevicePowerCycled
                | TriggerKind::InputDeviceManualConfigChanged
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputEventMatch {
    Press,
    Release,
    ShortPress,
    DoublePress,
    LongPressStart,
    LongPressRepeat,
    LongPressStop,
    ButtonFree,
    ButtonStuck,
    Movement,
    NoMovement,
    AnyEvent,
    Raw { data: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OccupancyState {
    Occupied,
    Vacant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossDirection {
    Above,
    Below,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerTransition {
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupAggregate {
    AnyOn,
    AllOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineTransition {
    ComesOnline,
    GoesOffline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideTransition {
    Starts,
    Clears,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    InputEvent {
        #[serde(flatten)]
        source: InputSelector,
        event: InputEventMatch,
    },
    InputOccupancy {
        #[serde(flatten)]
        source: InputRef,
        becomes: OccupancyState,
    },
    InputLightCross {
        #[serde(flatten)]
        source: InputRef,
        direction: CrossDirection,
        threshold: u16,
    },
    InputPositionChange {
        #[serde(flatten)]
        source: InputRef,
    },
    InputDevicePowerCycled {
        #[serde(flatten)]
        device: InputDeviceRef,
    },
    InputDeviceManualConfigChanged {
        #[serde(flatten)]
        device: InputDeviceRef,
    },
    AtTime {
        time: TimeOfDay,
        days: DaySet,
    },
    AtSolar {
        event: SolarEvent,
        offset_ms: i32,
    },
    Every {
        period_ms: DurationMs,
    },
    LampTurns {
        lamp: LampRef,
        to: PowerTransition,
    },
    LampLevelCross {
        lamp: LampRef,
        direction: CrossDirection,
        threshold: u8,
    },
    GroupBecomes {
        group: GroupRef,
        becomes: GroupAggregate,
    },
    SceneRecalled {
        adapter_id: u8,
        scene: u8,
    },
    DeviceOnlineTransition {
        device: DeviceRef,
        transition: OnlineTransition,
    },
    HclOverride {
        target: LightTarget,
        transition: OverrideTransition,
    },
    TimerFires {
        timer: String,
    },
    ControllerStarts,
    ControllerBecomesActive,
    RuleFails {
        rule: String,
    },
    HttpTrigger,
}

impl Trigger {
    pub fn kind(&self) -> TriggerKind {
        match self {
            Trigger::InputEvent { .. } => TriggerKind::InputEvent,
            Trigger::InputOccupancy { .. } => TriggerKind::InputOccupancy,
            Trigger::InputLightCross { .. } => TriggerKind::InputLightCross,
            Trigger::InputPositionChange { .. } => TriggerKind::InputPositionChange,
            Trigger::InputDevicePowerCycled { .. } => TriggerKind::InputDevicePowerCycled,
            Trigger::InputDeviceManualConfigChanged { .. } => {
                TriggerKind::InputDeviceManualConfigChanged
            }
            Trigger::AtTime { .. } => TriggerKind::AtTime,
            Trigger::AtSolar { .. } => TriggerKind::AtSolar,
            Trigger::Every { .. } => TriggerKind::Every,
            Trigger::LampTurns { .. } => TriggerKind::LampTurns,
            Trigger::LampLevelCross { .. } => TriggerKind::LampLevelCross,
            Trigger::GroupBecomes { .. } => TriggerKind::GroupBecomes,
            Trigger::SceneRecalled { .. } => TriggerKind::SceneRecalled,
            Trigger::DeviceOnlineTransition { .. } => TriggerKind::DeviceOnlineTransition,
            Trigger::HclOverride { .. } => TriggerKind::HclOverride,
            Trigger::TimerFires { .. } => TriggerKind::TimerFires,
            Trigger::ControllerStarts => TriggerKind::ControllerStarts,
            Trigger::ControllerBecomesActive => TriggerKind::ControllerBecomesActive,
            Trigger::RuleFails { .. } => TriggerKind::RuleFails,
            Trigger::HttpTrigger => TriggerKind::HttpTrigger,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_kind_names_align_with_discriminants() {
        for (i, kind) in TriggerKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, i, "ALL order must match discriminants");
            assert_eq!(kind.name(), TRIGGER_KIND_NAMES[i]);
        }
    }
}
