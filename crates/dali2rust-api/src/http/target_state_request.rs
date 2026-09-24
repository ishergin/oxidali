use std::collections::BTreeMap;

use serde::de::IgnoredAny;
use serde::Deserialize;

use crate::http::handlers::common::{json_err, parse_color_mode, parse_power};
use crate::http::lenient::{MaybeF64, MaybeObj, MaybeString, MaybeU64};
use crate::http::types::HttpResponse;
use dali2rust_contracts::msg::{ColorMode, ColorValue, LightSetpoint, PowerState};

pub const TARGET_STATE_ALLOWED_KEYS: &[&str] = &[
    "power",
    "level",
    "color_mode",
    "color_temperature_kelvin",
    "xy",
    "rgb",
    "rgbwaf",
    "transition",
];

pub const TARGET_STATE_RUNTIME_READONLY_KEYS: &[&str] = &[
    "status",
    "failure_status",
    "value_source",
    "last_seen_ms",
    "last_dapc_source",
    "error",
    "included",
    "setpoint",
    "waf",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetStateKeyError {
    UnknownField,
    UnsupportedField,
}

pub fn classify_target_state_request_key(key: &str) -> Result<(), TargetStateKeyError> {
    if TARGET_STATE_ALLOWED_KEYS
        .iter()
        .any(|allowed| *allowed == key)
    {
        Ok(())
    } else if TARGET_STATE_RUNTIME_READONLY_KEYS
        .iter()
        .any(|ro| *ro == key)
    {
        Err(TargetStateKeyError::UnsupportedField)
    } else {
        Err(TargetStateKeyError::UnknownField)
    }
}

#[derive(Debug, Deserialize)]
pub struct XyBody {
    x: Option<MaybeF64>,
    y: Option<MaybeF64>,
}

#[derive(Debug, Deserialize)]
pub struct RgbBody {
    r: Option<MaybeU64>,
    g: Option<MaybeU64>,
    b: Option<MaybeU64>,
}

#[derive(Debug, Deserialize)]
pub struct RgbwafBody {
    r: Option<MaybeU64>,
    g: Option<MaybeU64>,
    b: Option<MaybeU64>,
    w: Option<MaybeU64>,
    a: Option<MaybeU64>,
    f: Option<MaybeU64>,
}

#[derive(Debug, Deserialize)]
pub struct SetpointFields {
    power: Option<MaybeString>,
    level: Option<MaybeU64>,
    color_mode: Option<MaybeString>,
    color_temperature_kelvin: Option<MaybeU64>,
    xy: Option<MaybeObj<XyBody>>,
    rgb: Option<MaybeObj<RgbBody>>,
    rgbwaf: Option<MaybeObj<RgbwafBody>>,
}

impl SetpointFields {
    pub(crate) fn has_any_value(&self) -> bool {
        self.power.is_some()
            || self.level.is_some()
            || self.color_mode.is_some()
            || self.color_temperature_kelvin.is_some()
            || self.xy.is_some()
            || self.rgb.is_some()
            || self.rgbwaf.is_some()
    }

    pub(crate) fn power_present(&self) -> bool {
        self.power.is_some()
    }

    pub(crate) fn level_present(&self) -> bool {
        self.level.is_some()
    }

    pub(crate) fn color_value_present(&self, mode: ColorMode) -> bool {
        match mode {
            ColorMode::Cct => self.color_temperature_kelvin.is_some(),
            ColorMode::Xy => self.xy.is_some(),
            ColorMode::Rgb => self.rgb.is_some(),
            ColorMode::Rgbwaf => self.rgbwaf.is_some(),
            _ => false,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct TargetStateBody {
    #[serde(flatten)]
    setpoint: SetpointFields,
    #[allow(dead_code, reason = "`transition` is allowed and ignored (legacy-pinned): a known field so it never lands in `extra`, but its value is never read")]
    transition: Option<IgnoredAny>,
    #[serde(flatten)]
    extra: BTreeMap<String, IgnoredAny>,
}

pub fn parse_light_setpoint(
    body: &TargetStateBody,
    supports_color_mode: impl Fn(ColorMode) -> bool,
) -> Result<LightSetpoint, HttpResponse> {
    validate_target_state_keys(&body.extra)?;
    apply_setpoint_fields(&body.setpoint, supports_color_mode)
}

pub(crate) fn apply_setpoint_fields(
    fields: &SetpointFields,
    supports_color_mode: impl Fn(ColorMode) -> bool,
) -> Result<LightSetpoint, HttpResponse> {
    let mut sp = LightSetpoint {
        power: PowerState::Unknown,
        ..LightSetpoint::default()
    };
    apply_setpoint_power_level(fields, &mut sp)?;
    apply_setpoint_color_mode(fields, &supports_color_mode, &mut sp)?;
    apply_setpoint_cct(fields, &supports_color_mode, &mut sp)?;
    apply_setpoint_xy(fields, &supports_color_mode, &mut sp)?;
    apply_setpoint_rgb(fields, &supports_color_mode, &mut sp)?;
    apply_setpoint_rgbwaf(fields, &supports_color_mode, &mut sp)?;
    if sp.color.is_none() {
        sp.color = Some(ColorValue::default());
    }
    Ok(sp)
}

fn validate_target_state_keys(extra: &BTreeMap<String, IgnoredAny>) -> Result<(), HttpResponse> {
    for key in extra.keys() {
        match classify_target_state_request_key(key.as_str()) {
            Ok(()) => {}
            Err(TargetStateKeyError::UnknownField) => return Err(json_err(400, "unknown_field")),
            Err(TargetStateKeyError::UnsupportedField) => {
                return Err(json_err(422, "unsupported_field"));
            }
        }
    }
    Ok(())
}

fn apply_setpoint_power_level(
    fields: &SetpointFields,
    sp: &mut LightSetpoint,
) -> Result<(), HttpResponse> {
    if let Some(power) = &fields.power {
        let MaybeString::Valid(s) = power else {
            return Err(json_err(422, "invalid_value"));
        };
        sp.power = parse_power(s).ok_or_else(|| json_err(422, "invalid_enum"))?;
    }
    if let Some(level) = &fields.level {
        let MaybeU64::Valid(lu) = level else {
            return Err(json_err(422, "invalid_value"));
        };
        if *lu > 254 {
            return Err(json_err(422, "invalid_value"));
        }
        sp.level = *lu as u8;
    }
    Ok(())
}

fn apply_setpoint_color_mode(
    fields: &SetpointFields,
    supports_color_mode: &impl Fn(ColorMode) -> bool,
    sp: &mut LightSetpoint,
) -> Result<(), HttpResponse> {
    let Some(MaybeString::Valid(cm)) = &fields.color_mode else {
        return Ok(());
    };
    let e = parse_color_mode(cm).ok_or_else(|| json_err(422, "invalid_enum"))?;
    if !supports_color_mode(e) {
        return Err(json_err(422, "unsupported_capability"));
    }
    sp.color.get_or_insert(ColorValue::default()).mode = e;
    Ok(())
}

const MIN_COLOR_TEMPERATURE_KELVIN: u64 = 1000;
const MAX_COLOR_TEMPERATURE_KELVIN: u64 = 20_000;

fn apply_setpoint_cct(
    fields: &SetpointFields,
    supports_color_mode: &impl Fn(ColorMode) -> bool,
    sp: &mut LightSetpoint,
) -> Result<(), HttpResponse> {
    let Some(MaybeU64::Valid(k)) = &fields.color_temperature_kelvin else {
        return Ok(());
    };
    let k = *k;
    if !(MIN_COLOR_TEMPERATURE_KELVIN..=MAX_COLOR_TEMPERATURE_KELVIN).contains(&k) {
        return Err(json_err(422, "invalid_value"));
    }
    if !supports_color_mode(ColorMode::Cct) {
        return Err(json_err(422, "unsupported_capability"));
    }
    let cv = sp.color.get_or_insert(ColorValue::default());
    cv.mode = ColorMode::Cct;
    cv.color_temperature_kelvin = k as u16;
    Ok(())
}

fn apply_setpoint_xy(
    fields: &SetpointFields,
    supports_color_mode: &impl Fn(ColorMode) -> bool,
    sp: &mut LightSetpoint,
) -> Result<(), HttpResponse> {
    let Some(MaybeObj::Valid(xy)) = &fields.xy else {
        return Ok(());
    };
    let xf = xy
        .x
        .as_ref()
        .and_then(MaybeF64::valid)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    let yf = xy
        .y
        .as_ref()
        .and_then(MaybeF64::valid)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    if !(0.0..=1.0).contains(&xf) || !(0.0..=1.0).contains(&yf) {
        return Err(json_err(422, "invalid_value"));
    }
    if !supports_color_mode(ColorMode::Xy) {
        return Err(json_err(422, "unsupported_capability"));
    }
    let x = (xf * 65535.0) as u16;
    let y = (yf * 65535.0) as u16;
    if x == 0 && y == 0 {
        return Err(json_err(422, "invalid_value"));
    }
    let cv = sp.color.get_or_insert(ColorValue::default());
    cv.mode = ColorMode::Xy;
    cv.x = x;
    cv.y = y;
    Ok(())
}

fn apply_setpoint_rgbwaf(
    fields: &SetpointFields,
    supports_color_mode: &impl Fn(ColorMode) -> bool,
    sp: &mut LightSetpoint,
) -> Result<(), HttpResponse> {
    let Some(MaybeObj::Valid(body)) = &fields.rgbwaf else {
        return Ok(());
    };
    let channels = [&body.r, &body.g, &body.b, &body.w, &body.a, &body.f];
    let mut values = [0u8; 6];
    for (slot, channel) in values.iter_mut().zip(channels) {
        let raw = channel
            .as_ref()
            .and_then(MaybeU64::valid)
            .ok_or_else(|| json_err(422, "invalid_value"))?;
        *slot = u8::try_from(raw).map_err(|_| json_err(422, "invalid_value"))?;
    }
    if !supports_color_mode(ColorMode::Rgbwaf) {
        return Err(json_err(422, "unsupported_capability"));
    }
    let cv = sp.color.get_or_insert(ColorValue::default());
    cv.mode = ColorMode::Rgbwaf;
    [cv.r, cv.g, cv.b, cv.w, cv.a, cv.f] = values;
    Ok(())
}

fn apply_setpoint_rgb(
    fields: &SetpointFields,
    supports_color_mode: &impl Fn(ColorMode) -> bool,
    sp: &mut LightSetpoint,
) -> Result<(), HttpResponse> {
    let Some(MaybeObj::Valid(rgb)) = &fields.rgb else {
        return Ok(());
    };
    let ru = rgb
        .r
        .as_ref()
        .and_then(MaybeU64::valid)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    let gu = rgb
        .g
        .as_ref()
        .and_then(MaybeU64::valid)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    let bu = rgb
        .b
        .as_ref()
        .and_then(MaybeU64::valid)
        .ok_or_else(|| json_err(422, "invalid_value"))?;
    if ru > 255 || gu > 255 || bu > 255 {
        return Err(json_err(422, "invalid_value"));
    }
    if !supports_color_mode(ColorMode::Rgb) {
        return Err(json_err(422, "unsupported_capability"));
    }
    let cv = sp.color.get_or_insert(ColorValue::default());
    cv.mode = ColorMode::Rgb;
    cv.r = ru as u8;
    cv.g = gu as u8;
    cv.b = bu as u8;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(value: serde_json::Value) -> TargetStateBody {
        serde_json::from_value(value).expect("target-state body")
    }

    fn parse_ok(value: serde_json::Value) -> LightSetpoint {
        match parse_light_setpoint(&body(value), |_| true) {
            Ok(setpoint) => setpoint,
            Err(_) => panic!("expected successful parse"),
        }
    }

    fn parse_err_code(value: serde_json::Value, supports: impl Fn(ColorMode) -> bool) -> String {
        let err = match parse_light_setpoint(&body(value), supports) {
            Err(err) => err,
            Ok(_) => panic!("expected parse error"),
        };
        let body: serde_json::Value =
            serde_json::from_slice(&err.into_body_bytes()).expect("error json");
        body.get("error")
            .and_then(|v| v.as_str())
            .expect("error code")
            .to_string()
    }

    #[test]
    fn classify_allows_power_and_rejects_runtime_key() {
        assert!(classify_target_state_request_key("power").is_ok());
        assert_eq!(
            classify_target_state_request_key("last_seen_ms"),
            Err(TargetStateKeyError::UnsupportedField)
        );
        assert_eq!(
            classify_target_state_request_key("not_a_field"),
            Err(TargetStateKeyError::UnknownField)
        );
    }

    #[test]
    fn parse_light_setpoint_empty_body_sets_default_color_slot() {
        let sp = match parse_light_setpoint(&body(json!({})), |_| true) {
            Ok(s) => s,
            Err(_) => panic!("expected setpoint"),
        };
        assert_eq!(sp.power, PowerState::Unknown);
        assert!(sp.color.is_some());
    }

    #[test]
    fn parse_light_setpoint_unknown_field_errors() {
        assert_eq!(
            parse_err_code(json!({ "foo": 1 }), |_| true),
            "unknown_field"
        );
    }

    #[test]
    fn parse_light_setpoint_runtime_readonly_field_errors() {
        assert_eq!(
            parse_err_code(json!({ "status": { "raw": 0 } }), |_| true),
            "unsupported_field"
        );
    }

    #[test]
    fn parse_light_setpoint_unknown_vs_readonly_precedence_is_alphabetical() {
        assert_eq!(
            parse_err_code(json!({ "status": { "raw": 0 }, "aaa": 1 }), |_| true),
            "unknown_field"
        );
        assert_eq!(
            parse_err_code(json!({ "zzz": 1, "error": {} }), |_| true),
            "unsupported_field"
        );
    }

    #[test]
    fn parse_light_setpoint_rejects_invalid_power_enum() {
        assert_eq!(
            parse_err_code(json!({ "power": "sideways" }), |_| true),
            "invalid_enum"
        );
    }

    #[test]
    fn parse_light_setpoint_power_wrong_type_is_invalid_value() {
        assert_eq!(parse_err_code(json!({ "power": 5 }), |_| true), "invalid_value");
    }

    #[test]
    fn parse_light_setpoint_level_wrong_type_is_invalid_value() {
        assert_eq!(
            parse_err_code(json!({ "level": 1.5 }), |_| true),
            "invalid_value"
        );
        assert_eq!(
            parse_err_code(json!({ "level": "5" }), |_| true),
            "invalid_value"
        );
    }

    #[test]
    fn parse_light_setpoint_non_string_color_mode_is_ignored() {
        let sp = match parse_light_setpoint(&body(json!({ "color_mode": 5 })), |_| false) {
            Ok(s) => s,
            Err(_) => panic!("non-string color_mode must be ignored"),
        };
        assert_eq!(sp.color.expect("default color slot").mode, ColorMode::default());
    }

    #[test]
    fn parse_light_setpoint_null_fields_equal_absent() {
        let sp = parse_ok(json!({
            "power": null,
            "level": null,
            "color_mode": null,
            "color_temperature_kelvin": null,
            "xy": null,
            "rgb": null
        }));
        assert_eq!(sp.power, PowerState::Unknown);
        assert_eq!(sp.level, 0);
        assert_eq!(sp.color.expect("default color slot").mode, ColorMode::default());
    }

    #[test]
    fn parse_light_setpoint_unsupported_capability_for_cct() {
        assert_eq!(
            parse_err_code(json!({ "color_temperature_kelvin": 3000 }), |_| false),
            "unsupported_capability"
        );
    }

    #[test]
    fn parse_light_setpoint_rejects_level_above_254() {
        assert_eq!(
            parse_err_code(json!({ "level": 255 }), |_| true),
            "invalid_value"
        );
    }

    #[test]
    fn parse_light_setpoint_rejects_cct_above_u16() {
        assert_eq!(
            parse_err_code(json!({ "color_temperature_kelvin": 70000 }), |_| true),
            "invalid_value"
        );
    }

    #[test]
    fn parse_light_setpoint_rejects_cct_that_would_truncate_in_mirek() {
        for kelvin in [1, 15, 999] {
            assert_eq!(
                parse_err_code(json!({ "color_temperature_kelvin": kelvin }), |_| true),
                "invalid_value",
                "{kelvin} K must not reach the wire"
            );
        }
    }

    #[test]
    fn parse_light_setpoint_accepts_the_physical_cct_range() {
        for kelvin in [1000u64, 2700, 6500, 20_000] {
            let sp = parse_light_setpoint(&body(json!({ "color_temperature_kelvin": kelvin })), |_| true)
                .unwrap_or_else(|_| panic!("{kelvin} K is a legitimate setpoint"));
            assert_eq!(
                sp.color.expect("color slot").color_temperature_kelvin,
                kelvin as u16
            );
        }
    }

    #[test]
    fn parse_light_setpoint_rejects_xy_out_of_range() {
        assert_eq!(
            parse_err_code(json!({ "xy": { "x": 1.1, "y": 0.5 } }), |_| true),
            "invalid_value"
        );
    }

    #[test]
    fn parse_light_setpoint_rejects_the_xy_origin_sentinel() {
        assert_eq!(
            parse_err_code(json!({ "xy": { "x": 0.0, "y": 0.0 } }), |_| true),
            "invalid_value"
        );
        assert_eq!(
            parse_err_code(json!({ "xy": { "x": 0.00001, "y": 0.00001 } }), |_| true),
            "invalid_value"
        );
    }

    #[test]
    fn parse_light_setpoint_rejects_rgb_component_above_255() {
        assert_eq!(
            parse_err_code(json!({ "rgb": { "r": 256, "g": 1, "b": 2 } }), |_| true),
            "invalid_value"
        );
    }

    #[test]
    fn parse_light_setpoint_builds_expected_cct_setpoint() {
        let sp = parse_ok(json!({
            "power": "on",
            "level": 77,
            "color_temperature_kelvin": 2700
        }));
        assert_eq!(sp.power, PowerState::On);
        assert_eq!(sp.level, 77);
        let color = sp.color.expect("cct color");
        assert_eq!(color.mode, ColorMode::Cct);
        assert_eq!(color.color_temperature_kelvin, 2700);
    }

    #[test]
    fn parse_light_setpoint_builds_expected_xy_setpoint() {
        let sp = parse_ok(json!({
            "xy": { "x": 0.5, "y": 0.25 }
        }));
        let color = sp.color.expect("xy color");
        assert_eq!(color.mode, ColorMode::Xy);
        assert_eq!(color.x, 32767);
        assert_eq!(color.y, 16383);
    }

    #[test]
    fn parse_light_setpoint_builds_expected_rgb_setpoint() {
        let sp = parse_ok(json!({
            "rgb": { "r": 10, "g": 20, "b": 30 }
        }));
        let color = sp.color.expect("rgb color");
        assert_eq!(color.mode, ColorMode::Rgb);
        assert_eq!((color.r, color.g, color.b), (10, 20, 30));
    }

    #[test]
    fn target_state_body_rejects_duplicate_key() {
        let r = serde_json::from_slice::<TargetStateBody>(br#"{"level":1,"level":2}"#);
        assert!(r.is_err(), "duplicate key must fail the typed parse");
    }
}
