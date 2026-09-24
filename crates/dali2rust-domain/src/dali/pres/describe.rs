use crate::dali::dev103::{
    decode_event, instance_type, ButtonEvent, EventSource, ForwardFrame24, InputEvent,
};
use crate::dali::devices::DeviceType;
use crate::dali::net::address::{decode_wire_address, DaliAddress};
use crate::dali::pres::opcode::{
    CONFIG_ADD_TO_GROUP_BASE, CONFIG_REMOVE_FROM_GROUP_BASE, CONFIG_REMOVE_SCENE_BASE,
    CONFIG_SET_SCENE_BASE, EXTENDED_START, GO_TO_SCENE_BASE, QUERY_SCENE_BASE,
};
use crate::dali::pres::special::SpecialCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameTarget {
    Short(u8),
    Group(u8),
    Broadcast,
    BroadcastUnaddressed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameDescription {
    pub target: Option<FrameTarget>,
    pub name: String,
    pub detail: Option<String>,
    pub is_query: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DescribeContext {
    pub enabled_device_type: Option<u8>,
}

pub const ENABLE_DEVICE_TYPE_ADDRESS: u8 = SpecialCommand::EnableDeviceType(0).wire_bytes().0;

const DEVICE_TYPE_DT6_LED: u8 = DeviceType::Led.code();
const DEVICE_TYPE_DT8_COLOUR: u8 = DeviceType::Color.code();
const DAPC_SELECTOR_BIT: u8 = 0x01;

const SPECIAL_NAMES: &[(u8, &str)] = &[
    (0xA1, "TERMINATE"),
    (0xA3, "DTR0"),
    (0xA5, "INITIALISE"),
    (0xA7, "RANDOMISE"),
    (0xA9, "COMPARE"),
    (0xAB, "WITHDRAW"),
    (0xAD, "PING"),
    (0xB1, "SEARCHADDRH"),
    (0xB3, "SEARCHADDRM"),
    (0xB5, "SEARCHADDRL"),
    (0xB7, "PROGRAM SHORT ADDRESS"),
    (0xB9, "VERIFY SHORT ADDRESS"),
    (0xBB, "QUERY SHORT ADDRESS"),
    (0xBD, "PHYSICAL SELECTION"),
    (ENABLE_DEVICE_TYPE_ADDRESS, "ENABLE DEVICE TYPE"),
    (0xC3, "DTR1"),
    (0xC5, "DTR2"),
    (0xC7, "WRITE MEMORY LOCATION"),
    (0xC9, "WRITE MEMORY LOCATION NO REPLY"),
];

const SPECIAL_QUERIES: &[u8] = &[0xA9, 0xB9, 0xBB];

const OPCODE_NAMES: &[(u8, &str)] = &[
    (0x00, "OFF"),
    (0x01, "UP"),
    (0x02, "DOWN"),
    (0x03, "STEP UP"),
    (0x04, "STEP DOWN"),
    (0x05, "RECALL MAX LEVEL"),
    (0x06, "RECALL MIN LEVEL"),
    (0x07, "STEP DOWN AND OFF"),
    (0x08, "ON AND STEP UP"),
    (0x09, "ENABLE DAPC SEQUENCE"),
    (0x0A, "GO TO LAST ACTIVE LEVEL"),
    (0x20, "RESET"),
    (0x21, "STORE ACTUAL LEVEL IN DTR0"),
    (0x22, "SAVE PERSISTENT VARIABLES"),
    (0x23, "SET OPERATING MODE"),
    (0x24, "RESET MEMORY BANK"),
    (0x25, "IDENTIFY DEVICE"),
    (0x2A, "SET MAX LEVEL"),
    (0x2B, "SET MIN LEVEL"),
    (0x2C, "SET SYSTEM FAILURE LEVEL"),
    (0x2D, "SET POWER ON LEVEL"),
    (0x2E, "SET FADE TIME"),
    (0x2F, "SET FADE RATE"),
    (0x30, "SET EXTENDED FADE TIME"),
    (0x80, "SET SHORT ADDRESS"),
    (0x81, "ENABLE WRITE MEMORY"),
    (0x90, "QUERY STATUS"),
    (0x91, "QUERY CONTROL GEAR PRESENT"),
    (0x92, "QUERY LAMP FAILURE"),
    (0x93, "QUERY LAMP POWER ON"),
    (0x94, "QUERY LIMIT ERROR"),
    (0x95, "QUERY RESET STATE"),
    (0x96, "QUERY MISSING SHORT ADDRESS"),
    (0x97, "QUERY VERSION NUMBER"),
    (0x98, "QUERY CONTENT DTR0"),
    (0x99, "QUERY DEVICE TYPE"),
    (0x9A, "QUERY PHYSICAL MINIMUM"),
    (0x9B, "QUERY POWER FAILURE"),
    (0x9C, "QUERY CONTENT DTR1"),
    (0x9D, "QUERY CONTENT DTR2"),
    (0x9E, "QUERY OPERATING MODE"),
    (0x9F, "QUERY LIGHT SOURCE TYPE"),
    (0xA0, "QUERY ACTUAL LEVEL"),
    (0xA1, "QUERY MAX LEVEL"),
    (0xA2, "QUERY MIN LEVEL"),
    (0xA3, "QUERY POWER ON LEVEL"),
    (0xA4, "QUERY SYSTEM FAILURE LEVEL"),
    (0xA5, "QUERY FADE TIME / FADE RATE"),
    (0xA6, "QUERY MANUFACTURER SPECIFIC MODE"),
    (0xA7, "QUERY NEXT DEVICE TYPE"),
    (0xAA, "QUERY CONTROL GEAR FAILURE"),
    (0xA8, "QUERY EXTENDED FADE TIME"),
    (0xC0, "QUERY GROUPS 0-7"),
    (0xC1, "QUERY GROUPS 8-15"),
    (0xC2, "QUERY RANDOM ADDRESS H"),
    (0xC3, "QUERY RANDOM ADDRESS M"),
    (0xC4, "QUERY RANDOM ADDRESS L"),
    (0xC5, "READ MEMORY LOCATION"),
];

const EXTRA_QUERY_OPCODES: &[u8] = &[0xC5];

const DT8_NAMES: &[(u8, &str)] = &[
    (0xE0, "DT8 SET TEMPORARY X-COORDINATE"),
    (0xE1, "DT8 SET TEMPORARY Y-COORDINATE"),
    (0xE2, "DT8 ACTIVATE"),
    (0xE3, "DT8 X-COORDINATE STEP UP"),
    (0xE4, "DT8 X-COORDINATE STEP DOWN"),
    (0xE5, "DT8 Y-COORDINATE STEP UP"),
    (0xE6, "DT8 Y-COORDINATE STEP DOWN"),
    (0xE7, "DT8 SET TEMPORARY COLOUR TEMPERATURE"),
    (0xE8, "DT8 COLOUR TEMPERATURE STEP COOLER"),
    (0xE9, "DT8 COLOUR TEMPERATURE STEP WARMER"),
    (0xEA, "DT8 SET TEMPORARY PRIMARY N DIM LEVEL"),
    (0xEB, "DT8 SET TEMPORARY RGB DIM LEVEL"),
    (0xEC, "DT8 SET TEMPORARY WAF DIM LEVEL"),
    (0xED, "DT8 SET TEMPORARY RGBWAF CONTROL"),
    (0xEE, "DT8 COPY REPORT TO TEMPORARY"),
    (0xF0, "DT8 STORE TY PRIMARY N"),
    (0xF1, "DT8 STORE XY-COORDINATE PRIMARY N"),
    (0xF2, "DT8 STORE COLOUR TEMPERATURE LIMIT"),
    (0xF3, "DT8 STORE GEAR FEATURES/STATUS"),
    (0xF5, "DT8 ASSIGN COLOUR TO LINKED CHANNEL"),
    (0xF6, "DT8 START AUTO CALIBRATION"),
    (0xF7, "DT8 QUERY GEAR FEATURES/STATUS"),
    (0xF8, "DT8 QUERY COLOUR STATUS"),
    (0xF9, "DT8 QUERY COLOUR TYPE FEATURES"),
    (0xFA, "DT8 QUERY COLOUR VALUE"),
    (0xFB, "DT8 QUERY RGBWAF CONTROL"),
    (0xFC, "DT8 QUERY ASSIGNED COLOUR"),
    (0xFF, "DT8 QUERY EXTENDED VERSION NUMBER"),
];

const DT6_NAMES: &[(u8, &str)] = &[
    (0xE0, "DT6 REFERENCE SYSTEM POWER"),
    (0xE1, "DT6 ENABLE CURRENT PROTECTOR"),
    (0xE2, "DT6 DISABLE CURRENT PROTECTOR"),
    (0xE3, "DT6 SELECT DIMMING CURVE"),
    (0xE4, "DT6 STORE DTR AS FAST FADE TIME"),
    (0xED, "DT6 QUERY GEAR TYPE"),
    (0xEE, "DT6 QUERY DIMMING CURVE"),
    (0xEF, "DT6 QUERY POSSIBLE OPERATING MODES"),
    (0xF0, "DT6 QUERY FEATURES"),
    (0xF1, "DT6 QUERY FAILURE STATUS"),
    (0xF2, "DT6 QUERY SHORT CIRCUIT"),
    (0xF3, "DT6 QUERY OPEN CIRCUIT"),
    (0xF4, "DT6 QUERY LOAD DECREASE"),
    (0xF5, "DT6 QUERY LOAD INCREASE"),
    (0xF6, "DT6 QUERY CURRENT PROTECTOR ACTIVE"),
    (0xF7, "DT6 QUERY THERMAL SHUTDOWN"),
    (0xF8, "DT6 QUERY THERMAL OVERLOAD"),
    (0xF9, "DT6 QUERY REFERENCE RUNNING"),
    (0xFA, "DT6 QUERY REFERENCE MEASUREMENT FAILED"),
    (0xFB, "DT6 QUERY CURRENT PROTECTOR ENABLED"),
    (0xFC, "DT6 QUERY OPERATING MODE"),
    (0xFD, "DT6 QUERY FAST FADE TIME"),
    (0xFE, "DT6 QUERY MIN FAST FADE TIME"),
    (0xFF, "DT6 QUERY EXTENDED VERSION NUMBER"),
];

// IEC 62386-207 §11.3.4.2
const DT6_EXTENDED_QUERY_START: u8 = 0xED;
const DT8_EXTENDED_QUERY_START: u8 = 0xF7;

const EXTENDED_QUERY_START_UNKNOWN: u8 = DT6_EXTENDED_QUERY_START;

fn lookup(table: &[(u8, &'static str)], key: u8) -> Option<&'static str> {
    table
        .iter()
        .find_map(|&(k, name)| (k == key).then_some(name))
}

fn contains(table: &[u8], key: u8) -> bool {
    table.contains(&key)
}

pub fn decode_target(address: u8) -> Option<FrameTarget> {
    match decode_wire_address(address).ok()? {
        DaliAddress::Short(short) => Some(FrameTarget::Short(short)),
        DaliAddress::Group(group) => Some(FrameTarget::Group(group)),
        DaliAddress::Broadcast => Some(FrameTarget::Broadcast),
        DaliAddress::BroadcastUnaddressed => Some(FrameTarget::BroadcastUnaddressed),
    }
}

fn ranged_opcode_name(opcode: u8) -> Option<String> {
    let nibble = opcode & 0x0F;
    match opcode {
        GO_TO_SCENE_BASE..=0x1F => Some(format!("GO TO SCENE {nibble}")),
        CONFIG_SET_SCENE_BASE..=0x4F => Some(format!("SET SCENE {nibble}")),
        CONFIG_REMOVE_SCENE_BASE..=0x5F => Some(format!("REMOVE FROM SCENE {nibble}")),
        CONFIG_ADD_TO_GROUP_BASE..=0x6F => Some(format!("ADD TO GROUP {nibble}")),
        CONFIG_REMOVE_FROM_GROUP_BASE..=0x7F => Some(format!("REMOVE FROM GROUP {nibble}")),
        QUERY_SCENE_BASE..=0xBF => Some(format!("QUERY SCENE LEVEL {nibble}")),
        _ => None,
    }
}

fn extended_opcode_name(opcode: u8, ctx: DescribeContext) -> (String, Option<String>) {
    match ctx.enabled_device_type {
        Some(DEVICE_TYPE_DT8_COLOUR) => named_or_hex(DT8_NAMES, opcode, "DT8"),
        Some(DEVICE_TYPE_DT6_LED) => named_or_hex(DT6_NAMES, opcode, "DT6"),
        Some(other) => (
            format!("EXTENDED 0x{opcode:02X}"),
            Some(format!("device type {other}")),
        ),
        None => (
            format!("EXTENDED 0x{opcode:02X}"),
            Some("no ENABLE DEVICE TYPE seen".to_string()),
        ),
    }
}

fn named_or_hex(
    table: &[(u8, &'static str)],
    opcode: u8,
    prefix: &str,
) -> (String, Option<String>) {
    match lookup(table, opcode) {
        Some(name) => (name.to_string(), None),
        None => (format!("{prefix} 0x{opcode:02X}"), None),
    }
}

fn is_query_command_byte(opcode: u8, ctx: DescribeContext) -> bool {
    if opcode >= EXTENDED_START {
        let start = match ctx.enabled_device_type {
            Some(DEVICE_TYPE_DT8_COLOUR) => DT8_EXTENDED_QUERY_START,
            Some(DEVICE_TYPE_DT6_LED) => DT6_EXTENDED_QUERY_START,
            _ => EXTENDED_QUERY_START_UNKNOWN,
        };
        return opcode >= start;
    }
    crate::dali::pres::opcode::is_query_opcode(opcode) || contains(EXTRA_QUERY_OPCODES, opcode)
}

fn describe_special(address: u8, data: u8) -> FrameDescription {
    let name = lookup(SPECIAL_NAMES, address)
        .map(str::to_string)
        .unwrap_or_else(|| format!("SPECIAL 0x{address:02X}"));
    FrameDescription {
        target: None,
        name,
        detail: Some(format!("data=0x{data:02X}")),
        is_query: contains(SPECIAL_QUERIES, address),
    }
}

fn describe_indirect(target: FrameTarget, opcode: u8, ctx: DescribeContext) -> FrameDescription {
    let (name, detail) = if opcode >= EXTENDED_START {
        extended_opcode_name(opcode, ctx)
    } else if let Some(ranged) = ranged_opcode_name(opcode) {
        (ranged, None)
    } else {
        match lookup(OPCODE_NAMES, opcode) {
            Some(name) => (name.to_string(), None),
            None => (format!("COMMAND 0x{opcode:02X}"), None),
        }
    };
    FrameDescription {
        target: Some(target),
        name,
        detail,
        is_query: is_query_command_byte(opcode, ctx),
    }
}

pub fn describe_forward16(address: u8, command: u8, ctx: DescribeContext) -> FrameDescription {
    let Some(target) = decode_target(address) else {
        return describe_special(address, command);
    };
    if address & DAPC_SELECTOR_BIT == 0 {
        return FrameDescription {
            target: Some(target),
            name: "DAPC".to_string(),
            detail: Some(dapc_detail(command)),
            is_query: false,
        };
    }
    describe_indirect(target, command, ctx)
}

fn dapc_detail(level: u8) -> String {
    match level {
        crate::dali::pres::opcode::DAPC_LEVEL_MASK => "level MASK (no change)".to_string(),
        0 => "level 0 (off)".to_string(),
        other => format!("level {other}"),
    }
}

pub fn describe_forward24(bytes: [u8; 3]) -> FrameDescription {
    let frame = ForwardFrame24::from_bytes(bytes);
    let raw = format!("0x{:02X} 0x{:02X} 0x{:02X}", bytes[0], bytes[1], bytes[2]);

    match decode_event(frame) {
        Some(InputEvent::PowerCycle {
            short_address,
            device_group,
        }) => FrameDescription {
            target: None,
            name: "POWER NOTIFICATION".to_string(),
            detail: Some(describe_power_cycle(short_address, device_group)),
            is_query: false,
        },
        Some(InputEvent::Instance { source, info }) => FrameDescription {
            target: None,
            name: describe_event_name(&source, info),
            detail: Some(describe_event_source(&source, info)),
            is_query: false,
        },
        None => FrameDescription {
            target: None,
            name: "24-BIT COMMAND".to_string(),
            detail: Some(format!("{raw} (IEC 62386-103 command)")),
            is_query: false,
        },
    }
}

fn describe_event_name(source: &EventSource, info: u16) -> String {
    if source.instance_type == Some(instance_type::PUSH_BUTTON) {
        if let Some(button) = ButtonEvent::from_info(info) {
            return format!("{button:?}");
        }
    }
    "INPUT NOTIFICATION".to_string()
}

fn describe_event_source(source: &EventSource, info: u16) -> String {
    let mut parts = vec![format!("scheme {}", source.scheme.code())];
    if let Some(a) = source.short_address {
        parts.push(format!("device {a}"));
    }
    if let Some(g) = source.device_group {
        parts.push(format!("device group {g}"));
    }
    if let Some(g) = source.instance_group {
        parts.push(format!("instance group {g}"));
    }
    if let Some(n) = source.instance_number {
        parts.push(format!("instance {n}"));
    }
    if let Some(t) = source.instance_type {
        parts.push(format!("type {t}"));
    }
    parts.push(format!("info 0x{info:03X}"));
    parts.join(", ")
}

fn describe_power_cycle(short_address: Option<u8>, device_group: Option<u8>) -> String {
    match (short_address, device_group) {
        (Some(a), Some(g)) => format!("device {a}, group {g}"),
        (Some(a), None) => format!("device {a}"),
        (None, Some(g)) => format!("group {g}, no short address"),
        (None, None) => "unaddressed device".to_string(),
    }
}

pub fn describe_backward8(value: u8) -> FrameDescription {
    FrameDescription {
        target: None,
        name: "REPLY".to_string(),
        detail: Some(format!("0x{value:02X} ({value})")),
        is_query: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_none() -> DescribeContext {
        DescribeContext::default()
    }

    #[test]
    fn dapc_names_its_level_and_its_target() {
        let d = describe_forward16(0x00, 120, ctx_none());
        assert_eq!(d.target, Some(FrameTarget::Short(0)));
        assert_eq!(d.name, "DAPC");
        assert_eq!(d.detail.as_deref(), Some("level 120"));
        assert!(!d.is_query);
    }

    #[test]
    fn dapc_mask_is_not_reported_as_level_255() {
        let d = describe_forward16(0x00, 0xFF, ctx_none());
        assert_eq!(d.detail.as_deref(), Some("level MASK (no change)"));
    }

    #[test]
    fn broadcast_go_to_scene_carries_the_scene_number() {
        let d = describe_forward16(0xFF, 0x13, ctx_none());
        assert_eq!(d.target, Some(FrameTarget::Broadcast));
        assert_eq!(d.name, "GO TO SCENE 3");
    }

    #[test]
    fn every_address_byte_classifies_the_same_as_the_wire_decoder() {
        for address in 0u8..=u8::MAX {
            let expected = match address {
                0x00..=0x7F => Some(FrameTarget::Short(address >> 1)),
                0x80..=0x9F => Some(FrameTarget::Group((address >> 1) & 0x0F)),
                0xFC | 0xFD => Some(FrameTarget::BroadcastUnaddressed),
                0xFE | 0xFF => Some(FrameTarget::Broadcast),
                _ => None,
            };
            assert_eq!(decode_target(address), expected, "address 0x{address:02X}");
        }
    }

    #[test]
    fn group_address_decodes_to_its_group() {
        let d = describe_forward16(0x8F, 0x00, ctx_none());
        assert_eq!(d.target, Some(FrameTarget::Group(7)));
        assert_eq!(d.name, "OFF");
    }

    #[test]
    fn ping_is_named_at_0xad() {
        let d = describe_forward16(0xAD, 0x00, ctx_none());
        assert_eq!(d.target, None);
        assert_eq!(d.name, "PING");
    }

    #[test]
    fn read_memory_location_is_named_and_marked_as_a_query() {
        let d = describe_forward16(0x01, 0xC5, ctx_none());
        assert_eq!(d.name, "READ MEMORY LOCATION");
        assert!(d.is_query);
    }

    #[test]
    fn dtr0_shows_the_operand_it_is_arming() {
        let d = describe_forward16(0xA3, 0x42, ctx_none());
        assert_eq!(d.name, "DTR0");
        assert_eq!(d.detail.as_deref(), Some("data=0x42"));
        assert!(!d.is_query);
    }

    #[test]
    fn an_unmodelled_opcode_still_gets_an_address_and_a_hex_name() {
        let d = describe_forward16(0x01, 0x3B, ctx_none());
        assert_eq!(d.target, Some(FrameTarget::Short(0)));
        assert_eq!(d.name, "COMMAND 0x3B");
    }

    #[test]
    fn shared_extended_opcodes_follow_the_armed_device_type() {
        let dt8 = DescribeContext {
            enabled_device_type: Some(DEVICE_TYPE_DT8_COLOUR),
        };
        let dt6 = DescribeContext {
            enabled_device_type: Some(DEVICE_TYPE_DT6_LED),
        };
        assert_eq!(
            describe_forward16(0x01, 0xF8, dt8).name,
            "DT8 QUERY COLOUR STATUS"
        );
        assert_eq!(
            describe_forward16(0x01, 0xF8, dt6).name,
            "DT6 QUERY THERMAL OVERLOAD"
        );
    }

    #[test]
    fn the_backward_window_of_a_shared_opcode_follows_the_armed_device_type() {
        let dt8 = DescribeContext {
            enabled_device_type: Some(DEVICE_TYPE_DT8_COLOUR),
        };
        let dt6 = DescribeContext {
            enabled_device_type: Some(DEVICE_TYPE_DT6_LED),
        };

        let store = describe_forward16(0x01, 0xF3, dt8);
        assert_eq!(store.name, "DT8 STORE GEAR FEATURES/STATUS");
        assert!(
            !store.is_query,
            "IEC 62386-209 §11.3.4.2 — 243 is a configuration command, not a query"
        );
        let same_byte = describe_forward16(0x01, 0xF3, dt6);
        assert_eq!(same_byte.name, "DT6 QUERY OPEN CIRCUIT");
        assert!(
            same_byte.is_query,
            "IEC 62386-207 §11.3.4.1 — DT6 queries start at 237"
        );

        let query = describe_forward16(0x01, 0xF7, dt8);
        assert_eq!(query.name, "DT8 QUERY GEAR FEATURES/STATUS");
        assert!(query.is_query, "247 is the first DT8 query");

        assert!(describe_forward16(0x01, 0xF3, ctx_none()).is_query);
    }

    #[test]
    fn an_extended_opcode_without_a_prelude_says_so_instead_of_guessing() {
        let d = describe_forward16(0x01, 0xF8, ctx_none());
        assert_eq!(d.name, "EXTENDED 0xF8");
        assert_eq!(d.detail.as_deref(), Some("no ENABLE DEVICE TYPE seen"));
    }

    #[test]
    fn verify_short_address_opens_a_backward_window() {
        assert!(describe_forward16(0xB9, 0x0B, ctx_none()).is_query);
        assert!(describe_forward16(0xBB, 0x00, ctx_none()).is_query);
        assert!(!describe_forward16(0xA3, 0x00, ctx_none()).is_query);
    }

    #[test]
    fn twenty_four_bit_commands_are_named_by_addressing_only() {
        let d = describe_forward24([0x01, 0x02, 0x03]);
        assert_eq!(d.name, "24-BIT COMMAND");
        assert!(d.detail.unwrap().contains("0x01 0x02 0x03"));
    }

    #[test]
    fn a_button_event_is_named_when_the_scheme_carries_the_instance_type() {
        let d = describe_forward24([0b0_000011_0, 0b0_00001_00, 0x02]);
        assert_eq!(d.name, "ShortPress");
        let detail = d.detail.unwrap();
        assert!(detail.contains("device 3"), "{detail}");
        assert!(detail.contains("scheme 1"), "{detail}");
    }

    #[test]
    fn an_event_under_scheme_2_stays_generic_because_the_type_is_not_on_the_wire() {
        let d = describe_forward24([0b0_000011_0, 0b1_00000_00, 0x02]);
        assert_eq!(d.name, "INPUT NOTIFICATION");
        assert!(d.detail.unwrap().contains("instance 0"));
    }

    #[test]
    fn a_twenty_four_bit_frame_never_claims_a_gear_target() {
        for bytes in [[0b0_000011_0u8, 0b1_00000_00, 0x02], [0xFE, 0xE0, 0x43]] {
            assert_eq!(describe_forward24(bytes).target, None, "{bytes:02X?}");
        }
    }

    #[test]
    fn a_power_notification_is_named_and_carries_its_device() {
        let d = describe_forward24([0xFE, 0b111_0_0000, 0b01_000101]);
        assert_eq!(d.name, "POWER NOTIFICATION");
        assert_eq!(d.detail.as_deref(), Some("device 5"));
    }

    #[test]
    fn backward_frames_report_both_hex_and_decimal() {
        let d = describe_backward8(0x01);
        assert_eq!(d.name, "REPLY");
        assert_eq!(d.detail.as_deref(), Some("0x01 (1)"));
    }
}
