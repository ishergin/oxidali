use dali2rust_contracts::msg::{ColorMode, LightSetpoint};
use dali2rust_domain::registry::CapabilityFlagsView;
use serde_json::{Map, Value};

use super::brightness::ha_to_level;
use crate::http::target_state_request::{parse_light_setpoint, TargetStateBody};
use crate::http::types::HttpResponse;

pub fn caps_accept(caps: &CapabilityFlagsView, mode: ColorMode) -> bool {
    dali2rust_domain::registry::capability_supports_color_mode(*caps, mode)
}

pub fn caps_accept_for_group(caps: &CapabilityFlagsView, mode: ColorMode) -> bool {
    dali2rust_domain::registry::capability_accepts_color_mode(*caps, mode)
}

pub fn ha_light_command_to_setpoint(
    payload: &[u8],
    supports_color_mode: impl Fn(ColorMode) -> bool,
) -> Result<LightSetpoint, HttpResponse> {
    let raw: Value = serde_json::from_slice(payload)
        .map_err(|_| crate::http::handlers::common::json_err(400, "invalid_json"))?;
    let obj = raw
        .as_object()
        .ok_or_else(|| crate::http::handlers::common::json_err(400, "invalid_json"))?;
    let translated = Value::Object(translate(obj));
    let body: TargetStateBody = serde_json::from_value(translated)
        .map_err(|_| crate::http::handlers::common::json_err(422, "invalid_value"))?;
    parse_light_setpoint(&body, supports_color_mode)
}

fn translate(obj: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(state) = obj.get("state").and_then(Value::as_str) {
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
    use dali2rust_contracts::msg::PowerState;

    fn anything(_: ColorMode) -> bool {
        true
    }

    fn parse(payload: &str) -> LightSetpoint {
        match ha_light_command_to_setpoint(payload.as_bytes(), anything) {
            Ok(sp) => sp,
            Err(_) => panic!("expected {payload} to parse"),
        }
    }

    #[test]
    fn on_with_brightness_becomes_a_power_and_level_setpoint() {
        let sp = parse(r#"{"state":"ON","brightness":180}"#);
        assert_eq!(sp.power, PowerState::On);
        assert_eq!(sp.level, 180, "brightness_scale 254 makes this the identity");
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
        assert_eq!(parse(r#"{"state":"ON","brightness":255}"#).level, 254);
    }

    #[test]
    fn an_impossible_colour_temperature_is_still_refused() {
        assert!(ha_light_command_to_setpoint(br#"{"color_temp":99999}"#, anything).is_err());
    }
}
