use crate::cursor::Cursor;
use crate::lexer::{Pos, TokenKind};
use crate::parser::refs;
use crate::parser::timeval;
use dali2rust_rules_model::limits::{MAX_INPUT_VALUE_10BIT, MAX_LEVEL, MAX_SCENE};
use dali2rust_rules_model::{
    CompileError, CrossDirection, DurationMs, GroupAggregate, InputEventMatch, InputSelector,
    OccupancyState, OnlineTransition, OverrideTransition, PowerTransition, Trigger, TriggerKind,
};

pub const PARSED_TRIGGER_KINDS: &[TriggerKind] = &[
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

pub fn trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    let (word, pos) = c.expect_ident("a trigger")?;
    match word.as_str() {
        "input" => input_trigger(c),
        "at" => at_trigger(c),
        "every" => every_trigger(c),
        "lamp" => lamp_trigger(c),
        "group" => group_trigger(c),
        "scene" => scene_trigger(c),
        "device" => device_trigger(c),
        "hcl" => hcl_trigger(c),
        "timer" => timer_trigger(c),
        "controller" => controller_trigger(c),
        "rule" => rule_trigger(c),
        "http" => http_trigger(c),
        "mqtt" => mqtt_reserved(c, pos),
        _ => Err(pos.err(format!("unknown trigger \"{word}\""))),
    }
}

fn input_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    if c.accept_kw("device") {
        return input_device_trigger(c);
    }
    let sel_pos = c.here();
    let source = refs::input_selector(c)?;
    if c.accept_kw("is") {
        return Ok(Trigger::InputEvent { source, event: event_match(c)? });
    }
    if c.accept_kw("becomes") {
        return input_occupancy(c, source, sel_pos);
    }
    if c.accept(&TokenKind::Dot) {
        return input_property_trigger(c, source, sel_pos);
    }
    Err(c.err_here("expected `is`, `becomes` or a property after input(…)"))
}

fn require_instance(
    source: InputSelector,
    pos: Pos,
) -> Result<dali2rust_rules_model::InputRef, CompileError> {
    match source {
        InputSelector::Instance(input) => Ok(input),
        InputSelector::Group(_) => Err(pos.err(
            "this form addresses one instance — use dev=…, inst=… (group addressing is only for `is <event>`)",
        )),
    }
}

fn input_occupancy(
    c: &mut Cursor<'_>,
    source: InputSelector,
    sel_pos: Pos,
) -> Result<Trigger, CompileError> {
    let source = require_instance(source, sel_pos)?;
    let (word, pos) = c.expect_ident("occupied or vacant")?;
    let becomes = match word.as_str() {
        "occupied" => OccupancyState::Occupied,
        "vacant" => OccupancyState::Vacant,
        _ => return Err(pos.err(format!("expected occupied or vacant, got \"{word}\""))),
    };
    Ok(Trigger::InputOccupancy { source, becomes })
}

fn input_property_trigger(
    c: &mut Cursor<'_>,
    source: InputSelector,
    sel_pos: Pos,
) -> Result<Trigger, CompileError> {
    let source = require_instance(source, sel_pos)?;
    let (prop, pos) = c.expect_ident("light or position")?;
    match prop.as_str() {
        "light" => {
            c.expect_kw("crosses")?;
            let direction = cross_direction(c)?;
            let (threshold, _) =
                c.expect_int_in("light threshold", 0, i64::from(MAX_INPUT_VALUE_10BIT))?;
            Ok(Trigger::InputLightCross { source, direction, threshold: threshold as u16 })
        }
        "position" => {
            c.expect_kw("changes")?;
            Ok(Trigger::InputPositionChange { source })
        }
        _ => Err(pos.err(format!("unknown input trigger property \"{prop}\""))),
    }
}

fn input_device_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    let device = refs::input_device_ref(c)?;
    if c.accept_kw("power") {
        c.expect_kw("cycled")?;
        return Ok(Trigger::InputDevicePowerCycled { device });
    }
    if c.accept_kw("manual") {
        c.expect_kw("config")?;
        c.expect_kw("changed")?;
        return Ok(Trigger::InputDeviceManualConfigChanged { device });
    }
    Err(c.err_here("expected `power cycled` or `manual config changed`"))
}

fn event_match(c: &mut Cursor<'_>) -> Result<InputEventMatch, CompileError> {
    let (word, pos) = c.expect_ident("an input event")?;
    match word.as_str() {
        "press" => Ok(InputEventMatch::Press),
        "release" => Ok(InputEventMatch::Release),
        "short_press" => Ok(InputEventMatch::ShortPress),
        "double_press" => Ok(InputEventMatch::DoublePress),
        "long_press_start" => Ok(InputEventMatch::LongPressStart),
        "long_press_repeat" => Ok(InputEventMatch::LongPressRepeat),
        "long_press_stop" => Ok(InputEventMatch::LongPressStop),
        "button_free" => Ok(InputEventMatch::ButtonFree),
        "button_stuck" => Ok(InputEventMatch::ButtonStuck),
        "movement" => Ok(InputEventMatch::Movement),
        "no_movement" => Ok(InputEventMatch::NoMovement),
        "any_event" => Ok(InputEventMatch::AnyEvent),
        "event" => raw_event(c),
        _ => Err(pos.err(format!("unknown input event \"{word}\""))),
    }
}

fn raw_event(c: &mut Cursor<'_>) -> Result<InputEventMatch, CompileError> {
    c.expect(&TokenKind::LParen, "`(` after event")?;
    c.expect_kw("data")?;
    c.expect(&TokenKind::Assign, "`=` after data")?;
    let (data, _) = c.expect_int_in("event data", 0, i64::from(MAX_INPUT_VALUE_10BIT))?;
    c.expect(&TokenKind::RParen, "`)`")?;
    Ok(InputEventMatch::Raw { data: data as u16 })
}

fn at_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    if let Some(TokenKind::Time { .. }) = c.peek().map(|t| &t.kind) {
        let time = timeval::time_of_day(c)?;
        let days = if c.accept_kw("on") { timeval::day_set(c)? } else { timeval::all_days() };
        return Ok(Trigger::AtTime { time, days });
    }
    let (event, offset_ms) = timeval::solar_with_offset(c)?;
    Ok(Trigger::AtSolar { event, offset_ms })
}

fn every_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    let (ms, pos) = c.expect_duration("every period")?;
    if ms < dali2rust_rules_model::limits::MIN_EVERY_PERIOD_MS {
        return Err(pos.err("every period must be at least 1s"));
    }
    Ok(Trigger::Every { period_ms: DurationMs(ms) })
}

fn lamp_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    let lamp = refs::lamp_ref(c)?;
    if c.accept_kw("turns") {
        let (word, pos) = c.expect_ident("on or off")?;
        let to = match word.as_str() {
            "on" => PowerTransition::On,
            "off" => PowerTransition::Off,
            _ => return Err(pos.err(format!("expected on or off, got \"{word}\""))),
        };
        return Ok(Trigger::LampTurns { lamp, to });
    }
    c.expect(&TokenKind::Dot, "`turns` or `.level`")?;
    c.expect_kw("level")?;
    c.expect_kw("crosses")?;
    let direction = cross_direction(c)?;
    let (threshold, _) = c.expect_int_in("level threshold", 0, i64::from(MAX_LEVEL))?;
    Ok(Trigger::LampLevelCross { lamp, direction, threshold: threshold as u8 })
}

fn cross_direction(c: &mut Cursor<'_>) -> Result<CrossDirection, CompileError> {
    let (word, pos) = c.expect_ident("above or below")?;
    match word.as_str() {
        "above" => Ok(CrossDirection::Above),
        "below" => Ok(CrossDirection::Below),
        _ => Err(pos.err(format!("expected above or below, got \"{word}\""))),
    }
}

fn group_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    let group = refs::group_ref(c)?;
    c.expect_kw("becomes")?;
    let becomes = group_aggregate(c)?;
    Ok(Trigger::GroupBecomes { group, becomes })
}

pub(crate) fn group_aggregate(c: &mut Cursor<'_>) -> Result<GroupAggregate, CompileError> {
    let (word, pos) = c.expect_ident("any_on or all_off")?;
    match word.as_str() {
        "any_on" => Ok(GroupAggregate::AnyOn),
        "all_off" => Ok(GroupAggregate::AllOff),
        _ => Err(pos.err(format!("expected any_on or all_off, got \"{word}\""))),
    }
}

fn scene_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    c.expect(&TokenKind::LParen, "`(` after scene")?;
    let (scene, _) = c.expect_int_in("scene", 0, i64::from(MAX_SCENE))?;
    let adapter_id = match scene_adapter_clause(c)? {
        Some(a) => a,
        None => c.resolver.primary_adapter(),
    };
    c.expect(&TokenKind::RParen, "`)`")?;
    c.expect_kw("recalled")?;
    Ok(Trigger::SceneRecalled { adapter_id, scene: scene as u8 })
}

fn scene_adapter_clause(c: &mut Cursor<'_>) -> Result<Option<u8>, CompileError> {
    if !c.accept(&TokenKind::Comma) {
        return Ok(None);
    }
    c.expect_kw("adapter")?;
    c.expect(&TokenKind::Assign, "`=` after adapter")?;
    let (value, pos) = c.expect_int_in("adapter", 0, 255)?;
    if !c.resolver.adapter_exists(value as u8) {
        return Err(pos.err(format!("unknown adapter {value}")));
    }
    Ok(Some(value as u8))
}

fn device_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    let device = refs::device_ref(c)?;
    if c.accept_kw("goes") {
        c.expect_kw("offline")?;
        return Ok(Trigger::DeviceOnlineTransition { device, transition: OnlineTransition::GoesOffline });
    }
    c.expect_kw("comes")?;
    c.expect_kw("online")?;
    Ok(Trigger::DeviceOnlineTransition { device, transition: OnlineTransition::ComesOnline })
}

fn hcl_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    c.expect_kw("override")?;
    let (word, pos) = c.expect_ident("starts or clears")?;
    let transition = match word.as_str() {
        "starts" => OverrideTransition::Starts,
        "clears" => OverrideTransition::Clears,
        _ => return Err(pos.err(format!("expected starts or clears, got \"{word}\""))),
    };
    c.expect_kw("for")?;
    let target = refs::light_target(c)?;
    Ok(Trigger::HclOverride { target, transition })
}

fn timer_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    let timer = quoted_ref(c, "timer name")?;
    c.expect_kw("fires")?;
    Ok(Trigger::TimerFires { timer })
}

fn controller_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    if c.accept_kw("starts") {
        return Ok(Trigger::ControllerStarts);
    }
    c.expect_kw("becomes")?;
    c.expect_kw("active")?;
    Ok(Trigger::ControllerBecomesActive)
}

fn rule_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    let rule = quoted_ref(c, "rule name")?;
    c.expect_kw("fails")?;
    Ok(Trigger::RuleFails { rule })
}

fn http_trigger(c: &mut Cursor<'_>) -> Result<Trigger, CompileError> {
    c.expect_kw("trigger")?;
    Ok(Trigger::HttpTrigger)
}

fn mqtt_reserved(c: &mut Cursor<'_>, pos: Pos) -> Result<Trigger, CompileError> {
    let _topic = c.expect_string("mqtt topic")?;
    if c.accept_kw("is") {
        let _payload = c.expect_string("mqtt payload")?;
    }
    Err(pos.err("`when mqtt` is reserved: external triggers arrive in stage 2 (ADR-016 A11)"))
}

pub(crate) fn quoted_ref(c: &mut Cursor<'_>, what: &str) -> Result<String, CompileError> {
    c.expect(&TokenKind::LParen, "`(`")?;
    let (name, _) = refs::checked_name(c, what)?;
    c.expect(&TokenKind::RParen, "`)`")?;
    Ok(name)
}
