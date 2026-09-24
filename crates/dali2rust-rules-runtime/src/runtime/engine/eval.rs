use dali2rust_rules_model::{
    Cmp, Condition, GroupAggregate, HclState, Reading, RuleSet, SolarEvent, TimeBound, ValueExpr,
    VarOperand, VarValue, Weekday,
};

use super::state::{EventCtx, Volatile, INPUT_VALUE_MASK_10BIT};
use super::world::WorldSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unevaluable;

pub(crate) type EvalResult<T> = Result<T, Unevaluable>;

pub(crate) struct EvalEnv<'a> {
    pub world: &'a WorldSnapshot,
    pub vol: &'a Volatile,
    pub rules: &'a RuleSet,
    pub enabled: &'a [bool],
    pub ctx: &'a EventCtx,
}

pub(crate) fn eval_value(env: &EvalEnv<'_>, expr: &ValueExpr) -> EvalResult<i64> {
    match expr {
        ValueExpr::Literal(v) => Ok(*v),
        ValueExpr::Reading(r) => eval_reading(env, r),
        ValueExpr::Offset { reading, delta } => {
            Ok(eval_reading(env, reading)?.saturating_add(i64::from(*delta)))
        }
    }
}

fn eval_reading(env: &EvalEnv<'_>, reading: &Reading) -> EvalResult<i64> {
    match reading {
        Reading::LampLevel { lamp }
        | Reading::LampCct { lamp }
        | Reading::LampIsOn { lamp }
        | Reading::LampLastLevel { lamp } => lamp_reading(env, reading, lamp.adapter_id, lamp.id),
        Reading::GroupAnyOn { group }
        | Reading::GroupAllOff { group }
        | Reading::GroupMemberCount { group } => {
            group_reading(env, reading, group.adapter_id, group.id)
        }
        Reading::InputOccupied { input }
        | Reading::InputLight { input }
        | Reading::InputPosition { input }
        | Reading::InputLastEventAge { input } => input_reading(env, reading, input),
        Reading::TimeNow | Reading::TimeHour | Reading::TimeMinute | Reading::TimeWeekday => {
            time_reading(env, reading)
        }
        Reading::SunRise | Reading::SunSet | Reading::SunIsUp => sun_reading(env, reading),
        Reading::Var { name } => var_int(env, name),
        Reading::EventValue => env.ctx.value.ok_or(Unevaluable),
        Reading::EventDevice => env.ctx.device.ok_or(Unevaluable),
        Reading::EventInstance => env.ctx.instance.ok_or(Unevaluable),
        Reading::EventOption => env.ctx.option.ok_or(Unevaluable),
    }
}

fn lamp_reading(env: &EvalEnv<'_>, reading: &Reading, adapter: u8, id: u16) -> EvalResult<i64> {
    let lamp = env.world.lamp(adapter, id).ok_or(Unevaluable)?;
    match reading {
        Reading::LampLevel { .. } => Ok(i64::from(lamp.level)),
        Reading::LampCct { .. } => lamp.cct_kelvin.map(i64::from).ok_or(Unevaluable),
        Reading::LampIsOn { .. } => Ok(i64::from(lamp.is_on)),
        _ => Ok(i64::from(lamp.last_level)),
    }
}

fn group_reading(env: &EvalEnv<'_>, reading: &Reading, adapter: u8, id: u16) -> EvalResult<i64> {
    let group = env.world.group(adapter, id).ok_or(Unevaluable)?;
    match reading {
        Reading::GroupAnyOn { .. } => Ok(i64::from(group.any_on)),
        Reading::GroupAllOff { .. } => Ok(i64::from(group.all_off)),
        _ => Ok(i64::from(group.member_count)),
    }
}

fn input_reading(
    env: &EvalEnv<'_>,
    reading: &Reading,
    input: &dali2rust_rules_model::InputRef,
) -> EvalResult<i64> {
    let st = env
        .world
        .input(input.adapter_id, input.device_short_address, input.instance_number)
        .ok_or(Unevaluable)?;
    match reading {
        Reading::InputOccupied { .. } => st.occupied.map(i64::from).ok_or(Unevaluable),
        Reading::InputLight { .. } => numeric_10bit(st.light),
        Reading::InputPosition { .. } => numeric_10bit(st.position),
        _ => st.last_event_age_ms.map(i64::from).ok_or(Unevaluable),
    }
}

fn numeric_10bit(value: Option<u16>) -> EvalResult<i64> {
    match value {
        Some(v) if v != INPUT_VALUE_MASK_10BIT => Ok(i64::from(v)),
        _ => Err(Unevaluable),
    }
}

fn time_reading(env: &EvalEnv<'_>, reading: &Reading) -> EvalResult<i64> {
    let wall = env.world.wall.ok_or(Unevaluable)?;
    match reading {
        Reading::TimeNow => Ok(i64::from(wall.minutes_of_day)),
        Reading::TimeHour => Ok(i64::from(wall.minutes_of_day / 60)),
        Reading::TimeMinute => Ok(i64::from(wall.minutes_of_day % 60)),
        _ => Ok(i64::from(wall.weekday)),
    }
}

fn sun_reading(env: &EvalEnv<'_>, reading: &Reading) -> EvalResult<i64> {
    let sun = env.world.sun.ok_or(Unevaluable)?;
    match reading {
        Reading::SunRise => Ok(i64::from(sun.sunrise_min)),
        Reading::SunSet => Ok(i64::from(sun.sunset_min)),
        _ => Ok(i64::from(sun_is_up(env)?)),
    }
}

fn sun_is_up(env: &EvalEnv<'_>) -> EvalResult<bool> {
    let wall = env.world.wall.ok_or(Unevaluable)?;
    let sun = env.world.sun.ok_or(Unevaluable)?;
    let t = wall.minutes_of_day;
    Ok(t >= sun.sunrise_min && t < sun.sunset_min)
}

fn var_int(env: &EvalEnv<'_>, name: &str) -> EvalResult<i64> {
    match env.vol.vars.get(name) {
        Some(VarValue::Int(v)) => Ok(*v),
        _ => Err(Unevaluable),
    }
}

pub(crate) fn resolve_time_bound(env: &EvalEnv<'_>, bound: &TimeBound) -> EvalResult<u16> {
    match bound {
        TimeBound::Clock(t) => Ok(u16::from(t.hour) * 60 + u16::from(t.minute)),
        TimeBound::Solar { event, offset_ms } => {
            let sun = env.world.sun.ok_or(Unevaluable)?;
            Ok(solar_minute(&sun, *event, *offset_ms))
        }
    }
}

pub(crate) fn solar_minute(sun: &super::world::SunTimes, event: SolarEvent, offset_ms: i32) -> u16 {
    let base = match event {
        SolarEvent::Sunrise => i32::from(sun.sunrise_min),
        SolarEvent::Sunset => i32::from(sun.sunset_min),
    };
    let minute = (base + offset_ms / 60_000).rem_euclid(1_440);
    minute as u16
}

fn minute_in_range(t: u16, start: u16, end: u16) -> bool {
    if start <= end {
        t >= start && t <= end
    } else {
        t >= start || t <= end
    }
}

fn compare_i64(cmp: Cmp, left: i64, right: i64) -> bool {
    match cmp {
        Cmp::Eq => left == right,
        Cmp::Ne => left != right,
        Cmp::Ge => left >= right,
        Cmp::Le => left <= right,
        Cmp::Gt => left > right,
        Cmp::Lt => left < right,
    }
}

fn compare_var(env: &EvalEnv<'_>, name: &str, cmp: Cmp, operand: &VarOperand) -> EvalResult<bool> {
    let held = env.vol.vars.get(name);
    match (held, operand) {
        (Some(VarValue::Text(t)), VarOperand::Text(rhs)) => Ok(match cmp {
            Cmp::Eq => t == rhs,
            Cmp::Ne => t != rhs,
            _ => false,
        }),
        (Some(VarValue::Int(v)), VarOperand::Value(expr)) => {
            Ok(compare_i64(cmp, *v, eval_value(env, expr)?))
        }
        _ => Ok(matches!(cmp, Cmp::Ne)),
    }
}

pub(crate) fn eval_condition(env: &EvalEnv<'_>, condition: &Condition) -> EvalResult<bool> {
    match condition {
        Condition::TimeInRange { start, end } => time_in_range_cond(env, start, end),
        Condition::DayIn { days } => day_in_cond(env, days),
        Condition::SunIs { up } => Ok(sun_is_up(env)? == *up),
        Condition::LampIs { lamp, on } => lamp_is_cond(env, lamp, *on),
        Condition::LampLevel { lamp, cmp, value } => {
            reading_cmp(env, &Reading::LampLevel { lamp: *lamp }, *cmp, value)
        }
        Condition::LampCct { lamp, cmp, value } => {
            reading_cmp(env, &Reading::LampCct { lamp: *lamp }, *cmp, value)
        }
        Condition::GroupState { group, state } => group_state_cond(env, group, *state),
        Condition::InputOccupancy { input, occupied } => {
            let left = eval_reading(env, &Reading::InputOccupied { input: *input })?;
            Ok((left != 0) == *occupied)
        }
        Condition::InputLight { input, above, threshold } => {
            input_light_cond(env, input, *above, threshold)
        }
        Condition::HclState { target, state } => Ok(hcl_state(env, target, *state)),
        Condition::DeviceOnline { device } => Ok(env
            .world
            .device(device.adapter_id, device.short_address)
            .is_some_and(|d| d.online)),
        Condition::RuleEnabled { rule } => Ok(rule_enabled(env, rule)),
        Condition::LastFiredOlderThan { rule, than_ms } => {
            Ok(last_fired_older(env, rule, than_ms.0))
        }
        Condition::TimerRunning { timer } => Ok(env.vol.timer(timer).is_some()),
        Condition::ControllerActive => Ok(env.world.controller_active),
        Condition::VarCompare { name, cmp, value } => compare_var(env, name, *cmp, value),
    }
}

fn time_in_range_cond(env: &EvalEnv<'_>, start: &TimeBound, end: &TimeBound) -> EvalResult<bool> {
    let wall = env.world.wall.ok_or(Unevaluable)?;
    let s = resolve_time_bound(env, start)?;
    let e = resolve_time_bound(env, end)?;
    Ok(minute_in_range(wall.minutes_of_day, s, e))
}

fn day_in_cond(env: &EvalEnv<'_>, days: &dali2rust_rules_model::DaySet) -> EvalResult<bool> {
    let wall = env.world.wall.ok_or(Unevaluable)?;
    let day = Weekday::ALL.get(usize::from(wall.weekday)).ok_or(Unevaluable)?;
    Ok(days.contains(*day))
}

fn lamp_is_cond(env: &EvalEnv<'_>, lamp: &dali2rust_rules_model::LampRef, on: bool) -> EvalResult<bool> {
    let state = env.world.lamp(lamp.adapter_id, lamp.id).ok_or(Unevaluable)?;
    Ok(state.is_on == on)
}

fn reading_cmp(env: &EvalEnv<'_>, reading: &Reading, cmp: Cmp, value: &ValueExpr) -> EvalResult<bool> {
    let left = eval_reading(env, reading)?;
    Ok(compare_i64(cmp, left, eval_value(env, value)?))
}

fn group_state_cond(
    env: &EvalEnv<'_>,
    group: &dali2rust_rules_model::GroupRef,
    state: GroupAggregate,
) -> EvalResult<bool> {
    let g = env.world.group(group.adapter_id, group.id).ok_or(Unevaluable)?;
    Ok(match state {
        GroupAggregate::AnyOn => g.any_on,
        GroupAggregate::AllOff => g.all_off,
    })
}

fn input_light_cond(
    env: &EvalEnv<'_>,
    input: &dali2rust_rules_model::InputRef,
    above: bool,
    threshold: &ValueExpr,
) -> EvalResult<bool> {
    let left = eval_reading(env, &Reading::InputLight { input: *input })?;
    let bound = eval_value(env, threshold)?;
    Ok(if above { left > bound } else { left < bound })
}

fn hcl_state(env: &EvalEnv<'_>, target: &dali2rust_rules_model::LightTarget, state: HclState) -> bool {
    env.world.hcl_target(target).is_some_and(|row| match state {
        HclState::Enabled => row.enabled,
        HclState::Overridden => row.overridden,
    })
}

fn rule_enabled(env: &EvalEnv<'_>, name: &str) -> bool {
    env.rules
        .rules
        .iter()
        .position(|r| r.name == name)
        .and_then(|idx| env.enabled.get(idx).copied())
        .unwrap_or(false)
}

fn last_fired_older(env: &EvalEnv<'_>, rule: &str, than_ms: u32) -> bool {
    match env.vol.last_activation_ms.get(rule) {
        Some(at) => env.world.now_ms.saturating_sub(*at) > u64::from(than_ms),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minute_ranges_wrap_through_midnight_inclusively() {
        assert!(minute_in_range(420, 420, 1_380));
        assert!(minute_in_range(1_380, 420, 1_380));
        assert!(!minute_in_range(419, 420, 1_380));
        assert!(minute_in_range(30, 1_380, 300));
        assert!(minute_in_range(1_400, 1_380, 300));
        assert!(!minute_in_range(400, 1_380, 300));
    }

    #[test]
    fn solar_minutes_apply_signed_offsets_and_wrap() {
        let sun = crate::runtime::engine::world::SunTimes {
            sunrise_min: 330,
            sunset_min: 1_260,
        };
        assert_eq!(solar_minute(&sun, SolarEvent::Sunrise, -900_000), 315);
        assert_eq!(solar_minute(&sun, SolarEvent::Sunset, 1_800_000), 1_290);
        assert_eq!(solar_minute(&sun, SolarEvent::Sunset, 11_400_000), 10);
    }
}
