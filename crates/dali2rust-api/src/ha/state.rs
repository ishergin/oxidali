use dali2rust_contracts::msg::{ColorMode, LightSetpoint, PowerState};
use serde_json::{json, Map, Value};

use super::brightness::level_to_ha;
use crate::http::physical_device_state::XyDto;

fn ha_color_mode(mode: ColorMode) -> Option<&'static str> {
    match mode {
        ColorMode::Cct => Some("color_temp"),
        ColorMode::Xy => Some("xy"),
        ColorMode::Rgb | ColorMode::Rgbwaf => Some("rgb"),
        _ => None,
    }
}

pub fn light_state_payload(setpoint: &LightSetpoint) -> Value {
    let mut m = Map::new();
    let on = matches!(setpoint.power, PowerState::On);
    m.insert("state".into(), json!(if on { "ON" } else { "OFF" }));
    if on && setpoint.level > 0 {
        m.insert("brightness".into(), json!(level_to_ha(setpoint.level)));
    }
    if let Some(color) = setpoint.color.as_ref() {
        if let Some(mode) = ha_color_mode(color.mode) {
            insert_color(&mut m, mode, color);
        }
    }
    Value::Object(m)
}

fn insert_color(m: &mut Map<String, Value>, mode: &'static str, color: &dali2rust_contracts::msg::ColorValue) {
    let value = match mode {
        "color_temp" => color.cct().map(|k| ("color_temp", json!(k))),
        "xy" => color.xy_wire().map(|(x, y)| {
            let xy = XyDto::from_wire(x, y);
            ("color", json!({"x": xy.x, "y": xy.y}))
        }),
        _ => Some((
            "color",
            json!({"r": color.r, "g": color.g, "b": color.b}),
        )),
    };
    if let Some((key, v)) = value {
        m.insert("color_mode".into(), json!(mode));
        m.insert(key.into(), v);
    }
}

pub fn group_state_payload(state: &dali2rust_domain::registry::HaGroupStateView) -> Value {
    let mut m = Map::new();
    let on = state.commanded && state.any_on;
    m.insert("state".into(), json!(if on { "ON" } else { "OFF" }));
    if let (true, Some(b)) = (on, state.brightness) {
        m.insert("brightness".into(), json!(level_to_ha(b)));
    }
    let colour = state.color_mode.and_then(ha_color_mode).and_then(|mode| {
        let value = match mode {
            "color_temp" => ("color_temp", json!(state.color_temperature_kelvin?)),
            "rgb" => {
                let (r, g, b) = state.rgb?;
                ("color", json!({"r": r, "g": g, "b": b}))
            }
            "xy" => {
                let (x, y) = state.xy?;
                ("color", json!({"x": x, "y": y}))
            }
            _ => return None,
        };
        Some((mode, value))
    });
    if let Some((mode, (key, value))) = colour {
        m.insert("color_mode".into(), json!(mode));
        m.insert(key.into(), value);
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::ColorValue;

    fn setpoint(power: PowerState, level: u8, color: Option<ColorValue>) -> LightSetpoint {
        LightSetpoint {
            power,
            level,
            color,
        }
    }

    #[test]
    fn an_on_lamp_reports_upper_case_state_and_its_arc_level_as_brightness() {
        let v = light_state_payload(&setpoint(PowerState::On, 180, None));
        assert_eq!(v["state"], "ON");
        assert_eq!(v["brightness"], 180);
    }

    fn group(mode: Option<dali2rust_contracts::msg::ColorMode>,
             kelvin: Option<u16>,
             rgb: Option<(u8, u8, u8)>) -> Value {
        group_state_payload(&dali2rust_domain::registry::HaGroupStateView {
            commanded: true,
            any_on: true,
            brightness: Some(200),
            color_mode: mode,
            color_temperature_kelvin: kelvin,
            rgb,
            xy: None,
        })
    }

    #[test]
    fn an_agreed_group_colour_is_reported_in_the_mode_it_was_agreed_in() {
        use dali2rust_contracts::msg::ColorMode;
        let v = group(Some(ColorMode::Rgb), None, Some((254, 0, 120)));
        assert_eq!(v["color_mode"], "rgb");
        assert_eq!(v["color"], json!({"r": 254, "g": 0, "b": 120}));
        assert!(v.get("color_temp").is_none(), "one mode, not both: {v}");

        let v = group(Some(ColorMode::Cct), Some(3000), None);
        assert_eq!(v["color_mode"], "color_temp");
        assert_eq!(v["color_temp"], 3000);
        assert!(v.get("color").is_none(), "one mode, not both: {v}");
    }

    #[test]
    fn a_group_whose_members_disagree_reports_no_colour_at_all() {
        let v = group(None, None, None);
        assert!(v.get("color_mode").is_none(), "{v}");
        assert!(v.get("color").is_none(), "{v}");
        assert!(v.get("color_temp").is_none(), "{v}");
        assert_eq!(v["state"], "ON", "the group is still on: {v}");
    }

    #[test]
    fn an_off_lamp_reports_no_brightness_at_all() {
        let v = light_state_payload(&setpoint(PowerState::Off, 180, None));
        assert_eq!(v["state"], "OFF");
        assert!(v.get("brightness").is_none());
    }

    #[test]
    fn switching_on_without_a_level_reports_on_and_no_brightness() {
        let v = light_state_payload(&setpoint(PowerState::On, 0, None));
        assert_eq!(v["state"], "ON");
        assert!(
            v.get("brightness").is_none(),
            "a recall states no brightness: {v}"
        );
    }

    #[test]
    fn an_unknown_power_reads_as_off_rather_than_as_a_lit_lamp() {
        let v = light_state_payload(&setpoint(PowerState::Unknown, 0, None));
        assert_eq!(v["state"], "OFF");
    }

    #[test]
    fn a_cct_lamp_reports_kelvin_under_the_colour_temp_mode() {
        let color = ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 3000,
            ..Default::default()
        };
        let v = light_state_payload(&setpoint(PowerState::On, 180, Some(color)));
        assert_eq!(v["color_mode"], "color_temp");
        assert_eq!(v["color_temp"], 3000);
        assert!(v.get("color_temp_kelvin").is_none(), "config flag, not a payload key");
    }

    #[test]
    fn a_sentinel_zero_colour_is_omitted_rather_than_published_as_zero_kelvin() {
        let color = ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: 0,
            ..Default::default()
        };
        let v = light_state_payload(&setpoint(PowerState::On, 180, Some(color)));
        assert!(v.get("color_temp").is_none());
        assert!(v.get("color_mode").is_none());
    }

    #[test]
    fn a_group_tile_needs_both_a_command_and_a_lit_member() {
        use dali2rust_domain::registry::HaGroupStateView;
        let lit = HaGroupStateView {
            any_on: true,
            brightness: Some(180),
            ..Default::default()
        };
        let v = group_state_payload(&lit);
        assert_eq!(v["state"], "OFF", "lit member, but nobody commanded the group");
        assert!(v.get("brightness").is_none(), "an OFF tile reports no brightness");

        let commanded_dark = HaGroupStateView {
            commanded: true,
            ..Default::default()
        };
        assert_eq!(group_state_payload(&commanded_dark)["state"], "OFF");

        let commanded_lit = HaGroupStateView {
            commanded: true,
            any_on: true,
            brightness: Some(180),
            ..Default::default()
        };
        let v = group_state_payload(&commanded_lit);
        assert_eq!(v["state"], "ON");
        assert_eq!(v["brightness"], 180);
    }

    #[test]
    fn an_xy_group_reports_its_agreed_chromaticity() {
        use dali2rust_domain::registry::HaGroupStateView;
        let v = group_state_payload(&HaGroupStateView {
            commanded: true,
            any_on: true,
            brightness: Some(120),
            color_mode: Some(ColorMode::Xy),
            xy: Some((0.45, 0.41)),
            ..Default::default()
        });
        assert_eq!(v["color_mode"], "xy");
        assert_eq!(v["color"], json!({"x": 0.45, "y": 0.41}));
    }

    #[test]
    fn an_rgb_lamp_reports_a_colour_object() {
        let color = ColorValue {
            mode: ColorMode::Rgb,
            r: 255,
            g: 180,
            b: 90,
            ..Default::default()
        };
        let v = light_state_payload(&setpoint(PowerState::On, 254, Some(color)));
        assert_eq!(v["color_mode"], "rgb");
        assert_eq!(v["color"], json!({"r": 255, "g": 180, "b": 90}));
        assert_eq!(v["brightness"], 254, "full DALI level is full HA brightness");
    }
}

#[must_use]
pub fn input_event_state_payload(
    kind: dali2rust_contracts::msg::InputEventKind,
    typed_value: u16,
) -> Option<String> {
    use dali2rust_contracts::msg::InputEventKind;
    match kind {
        InputEventKind::Button => {
            let event = dali2rust_domain::dali::dev103::ButtonEvent::from_info(typed_value)?;
            Some(format!("{{\"event_type\":\"{}\"}}", event.name()))
        }
        InputEventKind::Occupancy => {
            let occupied = typed_value & OCCUPANCY_OCCUPIED_BIT != 0;
            Some(if occupied { "ON".to_string() } else { "OFF".to_string() })
        }
        InputEventKind::Position | InputEventKind::Illuminance => Some(typed_value.to_string()),
        InputEventKind::Generic => None,
    }
}

const OCCUPANCY_OCCUPIED_BIT: u16 = 1 << 1;
