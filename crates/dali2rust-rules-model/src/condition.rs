use crate::refs::{DeviceRef, GroupRef, InputRef, LampRef, LightTarget};
use crate::time::{DurationMs, TimeBound, DaySet};
use crate::trigger::GroupAggregate;
use crate::value::{ValueExpr, VarValue};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionKind {
    TimeInRange,
    DayIn,
    SunIs,
    LampIs,
    LampLevel,
    LampCct,
    GroupState,
    InputOccupancy,
    InputLight,
    HclState,
    DeviceOnline,
    RuleEnabled,
    LastFiredOlderThan,
    TimerRunning,
    ControllerActive,
    VarCompare,
}

pub const CONDITION_KIND_COUNT: usize = 16;

const CONDITION_KIND_NAMES: [&str; CONDITION_KIND_COUNT] = [
    "time_in_range",
    "day_in",
    "sun_is",
    "lamp_is",
    "lamp_level",
    "lamp_cct",
    "group_state",
    "input_occupancy",
    "input_light",
    "hcl_state",
    "device_online",
    "rule_enabled",
    "last_fired_older_than",
    "timer_running",
    "controller_active",
    "var_compare",
];

impl ConditionKind {
    pub const ALL: [ConditionKind; CONDITION_KIND_COUNT] = [
        ConditionKind::TimeInRange,
        ConditionKind::DayIn,
        ConditionKind::SunIs,
        ConditionKind::LampIs,
        ConditionKind::LampLevel,
        ConditionKind::LampCct,
        ConditionKind::GroupState,
        ConditionKind::InputOccupancy,
        ConditionKind::InputLight,
        ConditionKind::HclState,
        ConditionKind::DeviceOnline,
        ConditionKind::RuleEnabled,
        ConditionKind::LastFiredOlderThan,
        ConditionKind::TimerRunning,
        ConditionKind::ControllerActive,
        ConditionKind::VarCompare,
    ];

    pub fn name(self) -> &'static str {
        CONDITION_KIND_NAMES[self as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cmp {
    Eq,
    Ne,
    Ge,
    Le,
    Gt,
    Lt,
}

impl Cmp {
    pub fn symbol(self) -> &'static str {
        match self {
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
            Cmp::Ge => ">=",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Lt => "<",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HclState {
    Enabled,
    Overridden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum VarOperand {
    Text(String),
    Value(ValueExpr),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Condition {
    TimeInRange {
        start: TimeBound,
        end: TimeBound,
    },
    DayIn {
        days: DaySet,
    },
    SunIs {
        up: bool,
    },
    LampIs {
        lamp: LampRef,
        on: bool,
    },
    LampLevel {
        lamp: LampRef,
        cmp: Cmp,
        value: ValueExpr,
    },
    LampCct {
        lamp: LampRef,
        cmp: Cmp,
        value: ValueExpr,
    },
    GroupState {
        group: GroupRef,
        state: GroupAggregate,
    },
    InputOccupancy {
        #[serde(flatten)]
        input: InputRef,
        occupied: bool,
    },
    InputLight {
        #[serde(flatten)]
        input: InputRef,
        above: bool,
        threshold: ValueExpr,
    },
    HclState {
        target: LightTarget,
        state: HclState,
    },
    DeviceOnline {
        device: DeviceRef,
    },
    RuleEnabled {
        rule: String,
    },
    LastFiredOlderThan {
        rule: String,
        than_ms: DurationMs,
    },
    TimerRunning {
        timer: String,
    },
    ControllerActive,
    VarCompare {
        name: String,
        cmp: Cmp,
        value: VarOperand,
    },
}

impl Condition {
    pub fn kind(&self) -> ConditionKind {
        match self {
            Condition::TimeInRange { .. } => ConditionKind::TimeInRange,
            Condition::DayIn { .. } => ConditionKind::DayIn,
            Condition::SunIs { .. } => ConditionKind::SunIs,
            Condition::LampIs { .. } => ConditionKind::LampIs,
            Condition::LampLevel { .. } => ConditionKind::LampLevel,
            Condition::LampCct { .. } => ConditionKind::LampCct,
            Condition::GroupState { .. } => ConditionKind::GroupState,
            Condition::InputOccupancy { .. } => ConditionKind::InputOccupancy,
            Condition::InputLight { .. } => ConditionKind::InputLight,
            Condition::HclState { .. } => ConditionKind::HclState,
            Condition::DeviceOnline { .. } => ConditionKind::DeviceOnline,
            Condition::RuleEnabled { .. } => ConditionKind::RuleEnabled,
            Condition::LastFiredOlderThan { .. } => ConditionKind::LastFiredOlderThan,
            Condition::TimerRunning { .. } => ConditionKind::TimerRunning,
            Condition::ControllerActive => ConditionKind::ControllerActive,
            Condition::VarCompare { .. } => ConditionKind::VarCompare,
        }
    }
}

impl From<VarValue> for VarOperand {
    fn from(value: VarValue) -> Self {
        match value {
            VarValue::Text(s) => VarOperand::Text(s),
            VarValue::Int(i) => VarOperand::Value(ValueExpr::Literal(i)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn condition_kind_names_align_with_discriminants() {
        for (i, kind) in ConditionKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, i, "ALL order must match discriminants");
            assert_eq!(kind.name(), CONDITION_KIND_NAMES[i]);
        }
    }
}
