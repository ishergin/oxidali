use crate::refs::{GroupRef, InputRef, LampRef};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    LampLevel,
    LampCct,
    LampIsOn,
    LampLastLevel,
    GroupAnyOn,
    GroupAllOff,
    GroupMemberCount,
    InputOccupied,
    InputLight,
    InputPosition,
    InputLastEventAge,
    TimeNow,
    TimeHour,
    TimeMinute,
    TimeWeekday,
    SunRise,
    SunSet,
    SunIsUp,
    Var,
    EventValue,
    EventDevice,
    EventInstance,
    EventOption,
}

pub const VALUE_KIND_COUNT: usize = 23;

const VALUE_KIND_NAMES: [&str; VALUE_KIND_COUNT] = [
    "lamp_level",
    "lamp_cct",
    "lamp_is_on",
    "lamp_last_level",
    "group_any_on",
    "group_all_off",
    "group_member_count",
    "input_occupied",
    "input_light",
    "input_position",
    "input_last_event_age",
    "time_now",
    "time_hour",
    "time_minute",
    "time_weekday",
    "sun_rise",
    "sun_set",
    "sun_is_up",
    "var",
    "event_value",
    "event_device",
    "event_instance",
    "event_option",
];

impl ValueKind {
    pub const ALL: [ValueKind; VALUE_KIND_COUNT] = [
        ValueKind::LampLevel,
        ValueKind::LampCct,
        ValueKind::LampIsOn,
        ValueKind::LampLastLevel,
        ValueKind::GroupAnyOn,
        ValueKind::GroupAllOff,
        ValueKind::GroupMemberCount,
        ValueKind::InputOccupied,
        ValueKind::InputLight,
        ValueKind::InputPosition,
        ValueKind::InputLastEventAge,
        ValueKind::TimeNow,
        ValueKind::TimeHour,
        ValueKind::TimeMinute,
        ValueKind::TimeWeekday,
        ValueKind::SunRise,
        ValueKind::SunSet,
        ValueKind::SunIsUp,
        ValueKind::Var,
        ValueKind::EventValue,
        ValueKind::EventDevice,
        ValueKind::EventInstance,
        ValueKind::EventOption,
    ];

    pub fn name(self) -> &'static str {
        VALUE_KIND_NAMES[self as usize]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "value", rename_all = "snake_case")]
pub enum Reading {
    LampLevel { lamp: LampRef },
    LampCct { lamp: LampRef },
    LampIsOn { lamp: LampRef },
    LampLastLevel { lamp: LampRef },
    GroupAnyOn { group: GroupRef },
    GroupAllOff { group: GroupRef },
    GroupMemberCount { group: GroupRef },
    InputOccupied { input: InputRef },
    InputLight { input: InputRef },
    InputPosition { input: InputRef },
    InputLastEventAge { input: InputRef },
    TimeNow,
    TimeHour,
    TimeMinute,
    TimeWeekday,
    SunRise,
    SunSet,
    SunIsUp,
    Var { name: String },
    EventValue,
    EventDevice,
    EventInstance,
    EventOption,
}

impl Reading {
    pub fn kind(&self) -> ValueKind {
        match self {
            Reading::LampLevel { .. } => ValueKind::LampLevel,
            Reading::LampCct { .. } => ValueKind::LampCct,
            Reading::LampIsOn { .. } => ValueKind::LampIsOn,
            Reading::LampLastLevel { .. } => ValueKind::LampLastLevel,
            Reading::GroupAnyOn { .. } => ValueKind::GroupAnyOn,
            Reading::GroupAllOff { .. } => ValueKind::GroupAllOff,
            Reading::GroupMemberCount { .. } => ValueKind::GroupMemberCount,
            Reading::InputOccupied { .. } => ValueKind::InputOccupied,
            Reading::InputLight { .. } => ValueKind::InputLight,
            Reading::InputPosition { .. } => ValueKind::InputPosition,
            Reading::InputLastEventAge { .. } => ValueKind::InputLastEventAge,
            Reading::TimeNow => ValueKind::TimeNow,
            Reading::TimeHour => ValueKind::TimeHour,
            Reading::TimeMinute => ValueKind::TimeMinute,
            Reading::TimeWeekday => ValueKind::TimeWeekday,
            Reading::SunRise => ValueKind::SunRise,
            Reading::SunSet => ValueKind::SunSet,
            Reading::SunIsUp => ValueKind::SunIsUp,
            Reading::Var { .. } => ValueKind::Var,
            Reading::EventValue => ValueKind::EventValue,
            Reading::EventDevice => ValueKind::EventDevice,
            Reading::EventInstance => ValueKind::EventInstance,
            Reading::EventOption => ValueKind::EventOption,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ValueExpr {
    Literal(i64),
    Reading(Reading),
    Offset {
        #[serde(flatten)]
        reading: Reading,
        delta: i32,
    },
}

impl ValueExpr {
    pub fn reading(&self) -> Option<&Reading> {
        match self {
            ValueExpr::Literal(_) => None,
            ValueExpr::Reading(r) | ValueExpr::Offset { reading: r, .. } => Some(r),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum VarValue {
    Int(i64),
    Text(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_kind_names_align_with_discriminants() {
        for (i, kind) in ValueKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, i, "ALL order must match discriminants");
            assert_eq!(kind.name(), VALUE_KIND_NAMES[i]);
        }
    }
}
