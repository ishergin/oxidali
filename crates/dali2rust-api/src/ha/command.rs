use dali2rust_contracts::msg::ColorMode;
use dali2rust_domain::registry::CapabilityFlagsView;
use serde_json::{Map, Value};

use super::brightness::ha_to_level;
use crate::http::target_state_request::{
    parse_target_request, TargetRequest, TargetStateBody, LEVEL_PRESETS,
};
use crate::http::types::HttpResponse;

const HA_COMMAND_MAX_DEPTH: usize = 2;

pub fn ha_effects() -> impl Iterator<Item = &'static str> {
    LEVEL_PRESETS.iter().map(|&(name, _)| name)
}

pub fn caps_accept(caps: &CapabilityFlagsView, mode: ColorMode) -> bool {
    dali2rust_domain::registry::capability_supports_color_mode(*caps, mode)
}

pub fn caps_accept_for_group(caps: &CapabilityFlagsView, mode: ColorMode) -> bool {
    dali2rust_domain::registry::capability_accepts_color_mode(*caps, mode)
}

pub fn ha_light_command_to_request(
    payload: &[u8],
    supports_color_mode: impl Fn(ColorMode) -> bool,
) -> Result<TargetRequest, HttpResponse> {
    if crate::json_depth::depth_exceeds(payload, HA_COMMAND_MAX_DEPTH) {
        return Err(crate::http::handlers::common::json_err(400, "invalid_json"));
    }
    let raw: Value = serde_json::from_slice(payload)
        .map_err(|_| crate::http::handlers::common::json_err(400, "invalid_json"))?;
    let obj = raw
        .as_object()
        .ok_or_else(|| crate::http::handlers::common::json_err(400, "invalid_json"))?;
    let translated = Value::Object(translate(obj));
    let body: TargetStateBody = serde_json::from_value(translated)
        .map_err(|_| crate::http::handlers::common::json_err(422, "invalid_value"))?;
    parse_target_request(&body, supports_color_mode)
}

fn translate(obj: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    let effect = obj.get("effect").filter(|effect| !effect.is_null());
    if let Some(effect) = effect {
        out.insert("recall".into(), effect.clone());
    }
    let state = obj.get("state").and_then(Value::as_str);
    if let Some(state) = state.filter(|state| effect.is_none() || !state.eq_ignore_ascii_case("ON")) {
        out.insert(
            "power".into(),
            Value::String(if state.eq_ignore_ascii_case("ON") {
                "on".into()
            } else {
                "off".into()
            }),
        );
    }
    if let Some(b) = obj.get("brightness").and_then(Value::as_u64) {
        out.insert(
            "level".into(),
            Value::from(ha_to_level(u8::try_from(b).unwrap_or(u8::MAX))),
        );
    }
    if let Some(k) = obj.get("color_temp").and_then(Value::as_u64) {
        out.insert("color_mode".into(), Value::String("cct".into()));
        out.insert("color_temperature_kelvin".into(), Value::from(k));
    }
    translate_color(obj, &mut out);
    out
}

fn translate_color(obj: &Map<String, Value>, out: &mut Map<String, Value>) {
    let Some(color) = obj.get("color").and_then(Value::as_object) else {
        return;
    };
    if let (Some(x), Some(y)) = (color.get("x"), color.get("y")) {
        out.insert("color_mode".into(), Value::String("xy".into()));
        let mut xy = Map::new();
        xy.insert("x".into(), x.clone());
        xy.insert("y".into(), y.clone());
        out.insert("xy".into(), Value::Object(xy));
        return;
    }
    if let (Some(r), Some(g), Some(b)) = (color.get("r"), color.get("g"), color.get("b")) {
        out.insert("color_mode".into(), Value::String("rgb".into()));
        let mut rgb = Map::new();
        rgb.insert("r".into(), r.clone());
        rgb.insert("g".into(), g.clone());
        rgb.insert("b".into(), b.clone());
        out.insert("rgb".into(), Value::Object(rgb));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::{LevelPreset, LightSetpoint, PowerState};

    fn anything(_: ColorMode) -> bool {
        true
    }

    fn parse(payload: &str) -> LightSetpoint {
        match ha_light_command_to_request(payload.as_bytes(), anything) {
            Ok(TargetRequest::Setpoint(sp)) => sp,
            _ => panic!("expected {payload} to parse as a setpoint"),
        }
    }

    #[test]
    fn an_effect_is_a_recall_and_the_on_beside_it_is_implied() {
        for (effect, preset) in [("max_level", LevelPreset::MaxLevel), ("min_level", LevelPreset::MinLevel)] {
            let payload = format!(r#"{{"state":"ON","effect":"{effect}"}}"#);
            assert_eq!(
                ha_light_command_to_request(payload.as_bytes(), anything).ok(),
                Some(TargetRequest::Recall(preset))
            );
        }
    }

    #[test]
    fn an_effect_beside_a_brightness_or_an_unknown_effect_is_refused() {
        assert!(ha_light_command_to_request(br#"{"effect":"max_level","brightness":40}"#, anything).is_err());
        assert!(ha_light_command_to_request(br#"{"state":"OFF","effect":"max_level"}"#, anything).is_err());
        assert!(ha_light_command_to_request(br#"{"effect":"colorloop"}"#, anything).is_err());
    }

    #[test]
    fn on_with_brightness_becomes_a_power_and_level_setpoint() {
        let sp = parse(r#"{"state":"ON","brightness":180}"#);
        assert_eq!(sp.power, PowerState::On);
        assert_eq!(sp.level, Some(180), "brightness_scale 254 makes this the identity");
    }

    #[test]
    fn off_is_off() {
        assert_eq!(parse(r#"{"state":"OFF"}"#).power, PowerState::Off);
    }

    #[test]
    fn a_colour_only_command_leaves_power_alone() {
        let sp = parse(r#"{"color_temp":3000}"#);
        assert_eq!(sp.power, PowerState::Unknown, "no power was requested");
        let color = sp.color.expect("colour staged");
        assert_eq!(color.mode, ColorMode::Cct);
        assert_eq!(color.color_temperature_kelvin, 3000);
    }

    #[test]
    fn the_config_flag_name_is_not_read_as_a_payload_key() {
        let sp = parse(r#"{"state":"ON","color_temp_kelvin":3000}"#);
        assert!(
            sp.color.as_ref().is_none_or(|c| c.mode != ColorMode::Cct),
            "a key HA never sends must not become colour: {:?}",
            sp.color
        );
    }

    #[test]
    fn a_colour_object_is_read_as_xy_or_rgb_by_its_keys() {
        let xy = parse(r#"{"color":{"x":0.45,"y":0.41}}"#).color.unwrap();
        assert_eq!(xy.mode, ColorMode::Xy);
        let rgb = parse(r#"{"color":{"r":255,"g":180,"b":90}}"#).color.unwrap();
        assert_eq!(rgb.mode, ColorMode::Rgb);
        assert_eq!((rgb.r, rgb.g, rgb.b), (255, 180, 90));
    }

    #[test]
    fn an_out_of_scale_brightness_is_clamped_rather_than_refused() {
        assert_eq!(parse(r#"{"state":"ON","brightness":255}"#).level, Some(254));
    }

    fn state_on_nested(levels: usize) -> String {
        format!(r#"{{"state":"ON","x":{}1{}}}"#, r#"{"y":"#.repeat(levels), "}".repeat(levels))
    }

    #[test]
    fn a_command_nested_past_the_colour_object_is_refused_before_parsing() {
        let at_limit = state_on_nested(HA_COMMAND_MAX_DEPTH - 1);
        assert!(ha_light_command_to_request(at_limit.as_bytes(), anything).is_ok());
        let past = state_on_nested(HA_COMMAND_MAX_DEPTH);
        assert!(ha_light_command_to_request(past.as_bytes(), anything).is_err());
    }

    #[test]
    fn an_impossible_colour_temperature_is_still_refused() {
        assert!(ha_light_command_to_request(br#"{"color_temp":99999}"#, anything).is_err());
    }

    fn refusal(payload: &[u8]) -> String {
        let Err(err) = ha_light_command_to_request(payload, anything) else {
            panic!("expected {} to be refused", String::from_utf8_lossy(payload));
        };
        let body: Value = serde_json::from_slice(&err.into_body_bytes()).expect("error json");
        body["error"].as_str().expect("error code").to_string()
    }

    fn rgb_of(payload: &str) -> (ColorMode, u8, u8, u8) {
        let color = parse(payload).color.expect("colour slot");
        (color.mode, color.r, color.g, color.b)
    }

    #[test]
    fn only_a_json_object_is_a_command() {
        for payload in [&b"[1]"[..], b"1", b"null", br#""ON""#, b"", br#"[{"state":"ON"}]"#] {
            assert_eq!(refusal(payload), "invalid_json", "{}", String::from_utf8_lossy(payload));
        }
    }

    #[test]
    fn a_repeated_key_takes_its_last_value() {
        assert_eq!(parse(r#"{"state":"ON","state":"OFF"}"#).power, PowerState::Off);
        assert_eq!(parse(r#"{"state":"OFF","state":5}"#).power, PowerState::Unknown);
        assert_eq!(parse(r#"{"brightness":40,"brightness":"x"}"#).level, None);
        assert_eq!(rgb_of(r#"{"color":{"r":1,"g":2,"b":3,"r":9}}"#), (ColorMode::Rgb, 9, 2, 3));
        assert_eq!(rgb_of(r#"{"color":{"r":1,"g":2,"b":3},"color":5}"#).0, ColorMode::None);
        assert_eq!(parse(r#"{"effect":"max_level","effect":null,"state":"ON"}"#).power, PowerState::On);
    }

    #[test]
    fn unknown_keys_are_ignored_and_an_escaped_key_is_its_name() {
        let sp = parse(r#"{"state":"ON","transition":2,"foo":{"bar":1},"baz":[1,"2"],"color_mode":"xy"}"#);
        assert_eq!((sp.power, sp.color.map(|c| c.mode)), (PowerState::On, Some(ColorMode::None)));
        assert_eq!(parse(r#"{"st\u0061te":"OFF"}"#).power, PowerState::Off);
    }

    #[test]
    fn a_state_that_is_not_on_is_off_and_one_that_is_not_text_is_absent() {
        assert_eq!(parse(r#"{"state":"on"}"#).power, PowerState::On);
        assert_eq!(parse(r#"{"state":"BLAH"}"#).power, PowerState::Off);
        for payload in [r#"{"state":1}"#, r#"{"state":true}"#, r#"{"state":null}"#, r#"{"state":["ON"]}"#] {
            assert_eq!(parse(payload).power, PowerState::Unknown, "{payload}");
        }
    }

    #[test]
    fn a_brightness_or_colour_temperature_that_is_not_an_unsigned_integer_is_absent() {
        for value in ["180.0", "-5", "\"180\"", "null", "18446744073709551616", "-0", "{}"] {
            assert_eq!(parse(&format!(r#"{{"brightness":{value}}}"#)).level, None, "{value}");
            let sp = parse(&format!(r#"{{"color_temp":{value}}}"#));
            assert_eq!(sp.color.map(|c| c.mode), Some(ColorMode::None), "{value}");
        }
        assert_eq!(parse(r#"{"brightness":300}"#).level, Some(254));
    }

    #[test]
    fn only_a_null_effect_is_absent() {
        assert_eq!(parse(r#"{"effect":null,"state":"ON"}"#).power, PowerState::On);
        assert_eq!(refusal(br#"{"effect":5,"state":"ON"}"#), "invalid_value");
        assert_eq!(refusal(br#"{"effect":{}}"#), "invalid_value");
        assert_eq!(refusal(br#"{"effect":"MAX_LEVEL"}"#), "invalid_enum");
    }

    #[test]
    fn a_colour_is_chosen_by_which_keys_it_carries() {
        assert_eq!(rgb_of(r#"{"color":"red"}"#).0, ColorMode::None);
        assert_eq!(rgb_of(r#"{"color":{"r":1,"g":2}}"#).0, ColorMode::None);
        assert_eq!(rgb_of(r#"{"color":{"x":0.3,"r":1,"g":2,"b":3}}"#), (ColorMode::Rgb, 1, 2, 3));
        assert_eq!(parse(r#"{"color":{"x":0.3,"y":0.3,"r":1,"g":2,"b":3}}"#).color.unwrap().mode, ColorMode::Xy);
        assert_eq!(rgb_of(r#"{"color":{"r":1,"g":2,"b":3,"h":1,"s":2}}"#), (ColorMode::Rgb, 1, 2, 3));
        assert_eq!(refusal(br#"{"color":{"x":null,"y":0.4}}"#), "invalid_value");
        assert_eq!(refusal(br#"{"color":{"r":"a","g":1,"b":2}}"#), "invalid_value");
        assert_eq!(refusal(br#"{"color":{"r":-1,"g":1,"b":2}}"#), "invalid_value");
        assert_eq!(refusal(br#"{"color":{"r":1.0,"g":1,"b":2}}"#), "invalid_value");
        assert_eq!(refusal(br#"{"color":{"r":256,"g":1,"b":2}}"#), "invalid_value");
        assert_eq!(refusal(br#"{"color":{"x":"a","y":0.4}}"#), "invalid_value");
        let xy = parse(r#"{"color":{"x":0,"y":1}}"#).color.unwrap();
        assert_eq!((xy.mode, xy.x, xy.y), (ColorMode::Xy, 0, 65535));
        assert_eq!(refusal(br#"{"color":{"x":-1,"y":0.5}}"#), "invalid_value");
        assert_eq!(refusal(br#"{"color":{"r":null,"g":1,"b":2}}"#), "invalid_value");
    }

    #[test]
    fn a_colour_beside_a_colour_temperature_wins_the_mode_and_keeps_the_kelvin() {
        let color = parse(r#"{"color_temp":3000,"color":{"r":1,"g":2,"b":3}}"#).color.unwrap();
        assert_eq!((color.mode, color.color_temperature_kelvin, color.b), (ColorMode::Rgb, 3000, 3));
        let color = parse(r#"{"color":{"x":0.5,"y":0.25},"color_temp":3000}"#).color.unwrap();
        assert_eq!((color.mode, color.color_temperature_kelvin, color.x), (ColorMode::Xy, 3000, 32767));
    }

    #[test]
    fn a_truncated_or_malformed_command_is_invalid_json_whatever_it_read_first() {
        for payload in [
            &br#"{"state":"ON","color":{"r":1,"g":2"#[..],
            br#"{"state":"ON","color":{"r":1,"g":2,"b":3}"#,
            br#"{"state":"ON"} x"#,
            br#"{"state":"ON",}"#,
            b"{\"state\":\"ON\",\"foo\":\"\xff\"}",
            br#"{"state":"ON","foo":"\ud800"}"#,
            br#"{"state":"ON","foo":["\ud800"]}"#,
            br#"{"state":"ON","foo":1e400}"#,
            br#"{"color":{"x":0.3,"r":1,"g":2,"b":3,"q":"\q"}}"#,
        ] {
            assert_eq!(refusal(payload), "invalid_json", "{}", String::from_utf8_lossy(payload));
        }
    }

    #[test]
    fn a_colour_object_of_any_width_is_read_and_one_nested_deeper_is_refused() {
        let wide: String = (0..2000).map(|i| format!(r#""k{i}":{i},"#)).collect();
        let payload = format!(r#"{{"color":{{{wide}"r":1,"g":2,"b":3}}}}"#);
        assert_eq!(rgb_of(&payload), (ColorMode::Rgb, 1, 2, 3));
        assert_eq!(refusal(br#"{"color":{"r":{"v":1},"g":2,"b":3}}"#), "invalid_json");
    }
}
