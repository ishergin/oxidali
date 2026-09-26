use crate::dali::dev103::frame::ForwardFrame24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EventScheme {
    Instance = 0,
    Device = 1,
    DeviceInstance = 2,
    DeviceGroup = 3,
    InstanceGroup = 4,
}

impl EventScheme {
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Instance),
            1 => Some(Self::Device),
            2 => Some(Self::DeviceInstance),
            3 => Some(Self::DeviceGroup),
            4 => Some(Self::InstanceGroup),
            _ => None,
        }
    }

    #[must_use]
    pub const fn identifies_device(self) -> bool {
        matches!(self, Self::Device | Self::DeviceInstance)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSource {
    pub scheme: EventScheme,
    pub short_address: Option<u8>,
    pub device_group: Option<u8>,
    pub instance_group: Option<u8>,
    pub instance_number: Option<u8>,
    pub instance_type: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Instance { source: EventSource, info: u16 },
    PowerCycle {
        short_address: Option<u8>,
        device_group: Option<u8>,
    },
}

pub const EVENT_INFO_MASK: u16 = 0x03FF;

// IEC 62386-103 Table 20
const POWER_NOTIFICATION_BYTE0: u8 = 0xFE;
const POWER_NOTIFICATION_BYTE1_TOP: u8 = 0xE0;

#[must_use]
pub fn decode_event(frame: ForwardFrame24) -> Option<InputEvent> {
    if frame.is_command() {
        return None;
    }
    let [b0, b1, b2] = frame.as_bytes();

    if b0 == POWER_NOTIFICATION_BYTE0 && (b1 & 0xE0) == POWER_NOTIFICATION_BYTE1_TOP {
        return Some(decode_power_cycle(b1, b2));
    }

    let info = (u16::from(b1 & 0x03) << 8) | u16::from(b2);
    let source = decode_source(b0, b1)?;
    Some(InputEvent::Instance { source, info })
}

// IEC 62386-103 Table 3
fn decode_source(b0: u8, b1: u8) -> Option<EventSource> {
    use EventScheme::{Device, DeviceGroup, DeviceInstance, Instance, InstanceGroup};

    let scheme = match ((b0 >> 6) & 0x03, (b1 >> 7) & 0x01) {
        (0 | 1, 0) => Device,
        (0 | 1, _) => DeviceInstance,
        (2, 1) => Instance,
        (2, _) => DeviceGroup,
        (_, 0) => InstanceGroup,
        _ => return None,
    };

    let upper = (b0 >> 1) & 0x1F;
    let lower = (b1 >> 2) & 0x1F;

    Some(EventSource {
        scheme,
        short_address: matches!(scheme, Device | DeviceInstance).then(|| (b0 >> 1) & 0x3F),
        device_group: matches!(scheme, DeviceGroup).then_some(upper),
        instance_group: matches!(scheme, InstanceGroup).then_some(upper),
        instance_number: matches!(scheme, Instance | DeviceInstance).then_some(lower),
        instance_type: match scheme {
            Instance => Some(upper),
            DeviceInstance => None,
            _ => Some(lower),
        },
    })
}

fn decode_power_cycle(b1: u8, b2: u8) -> InputEvent {
    // IEC 62386-103 Table 7
    let group_valid = (b1 >> 4) & 0x01 == 1;
    let group = ((b1 & 0x0F) << 1) | ((b2 >> 7) & 0x01);
    let address_valid = (b2 >> 6) & 0x01 == 1;
    InputEvent::PowerCycle {
        short_address: address_valid.then_some(b2 & 0x3F),
        device_group: group_valid.then_some(group),
    }
}

pub mod instance_type {
    pub const GENERIC: u8 = 0;
    pub const PUSH_BUTTON: u8 = 1;
    pub const ABSOLUTE_INPUT: u8 = 2;
    pub const OCCUPANCY: u8 = 3;
    pub const LIGHT_SENSOR: u8 = 4;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonEvent {
    Release,
    Press,
    ShortPress,
    DoublePress,
    LongPressStart,
    LongPressRepeat,
    LongPressStop,
    ButtonFree,
    ButtonStuck,
}

impl ButtonEvent {
    pub const ALL: [Self; 9] = [
        Self::Release,
        Self::Press,
        Self::ShortPress,
        Self::DoublePress,
        Self::LongPressStart,
        Self::LongPressRepeat,
        Self::LongPressStop,
        Self::ButtonFree,
        Self::ButtonStuck,
    ];

    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Release => 0x000,
            Self::Press => 0x001,
            Self::ShortPress => 0x002,
            Self::DoublePress => 0x005,
            Self::LongPressStart => 0x009,
            Self::LongPressRepeat => 0x00B,
            Self::LongPressStop => 0x00C,
            Self::ButtonFree => 0x00E,
            Self::ButtonStuck => 0x00F,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Press => "press",
            Self::ShortPress => "short_press",
            Self::DoublePress => "double_press",
            Self::LongPressStart => "long_press_start",
            Self::LongPressRepeat => "long_press_repeat",
            Self::LongPressStop => "long_press_stop",
            Self::ButtonFree => "button_free",
            Self::ButtonStuck => "button_stuck",
        }
    }

    #[must_use]
    pub const fn from_info(info: u16) -> Option<Self> {
        match info & EVENT_INFO_MASK {
            0x000 => Some(Self::Release),
            0x001 => Some(Self::Press),
            0x002 => Some(Self::ShortPress),
            0x005 => Some(Self::DoublePress),
            0x009 => Some(Self::LongPressStart),
            0x00B => Some(Self::LongPressRepeat),
            0x00C => Some(Self::LongPressStop),
            0x00E => Some(Self::ButtonFree),
            0x00F => Some(Self::ButtonStuck),
            _ => None,
        }
    }

    // IEC 62386-301 Table 2
    #[must_use]
    pub const fn is_pressed(self) -> bool {
        self.code() & 0x001 == 0x001
    }
}

// IEC 62386-303 §9.4.3
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OccupancyEvent {
    pub movement: bool,
    pub occupied: bool,
    pub still: bool,
    pub movement_based: bool,
}

impl OccupancyEvent {
    const MOVEMENT: u16 = 1 << 0;
    const OCCUPIED: u16 = 1 << 1;
    const STILL: u16 = 1 << 2;
    const MOVEMENT_BASED: u16 = 1 << 3;
    const RESERVED: u16 = 0x3F0;

    #[must_use]
    pub const fn from_info(info: u16) -> Option<Self> {
        if info & Self::RESERVED != 0 {
            return None;
        }
        Some(Self {
            movement: info & Self::MOVEMENT != 0,
            occupied: info & Self::OCCUPIED != 0,
            still: info & Self::STILL != 0,
            movement_based: info & Self::MOVEMENT_BASED != 0,
        })
    }

    #[must_use]
    pub const fn to_info(self) -> u16 {
        (if self.movement { Self::MOVEMENT } else { 0 })
            | (if self.occupied { Self::OCCUPIED } else { 0 })
            | (if self.still { Self::STILL } else { 0 })
            | (if self.movement_based { Self::MOVEMENT_BASED } else { 0 })
    }

    #[must_use]
    pub const fn is_transition(self) -> bool {
        !self.still
    }
}

pub mod occupancy_filter {
    pub const OCCUPIED: u8 = 1 << 0;
    pub const VACANT: u8 = 1 << 1;
    pub const REPEAT: u8 = 1 << 2;
    pub const MOVEMENT: u8 = 1 << 3;
    pub const NO_MOVEMENT: u8 = 1 << 4;

    pub const FACTORY_DEFAULT: u8 = OCCUPIED | VACANT;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MagnitudeEvent {
    pub raw: u16,
}

impl MagnitudeEvent {
    #[must_use]
    pub const fn from_info(info: u16) -> Self {
        Self {
            raw: info & EVENT_INFO_MASK,
        }
    }
}

pub mod magnitude_filter {
    pub const POSITION: u8 = 1 << 0;
    pub const ILLUMINANCE: u8 = 1 << 0;
    pub const FACTORY_DEFAULT: u8 = 1;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedInputEvent {
    Button(ButtonEvent),
    Occupancy(OccupancyEvent),
    Position(MagnitudeEvent),
    Illuminance(MagnitudeEvent),
    Generic { instance_type: Option<u8>, info: u16 },
}

#[must_use]
pub fn type_event(instance_type: Option<u8>, info: u16) -> TypedInputEvent {
    let generic = TypedInputEvent::Generic { instance_type, info };
    match instance_type {
        Some(instance_type::PUSH_BUTTON) => {
            ButtonEvent::from_info(info).map_or(generic, TypedInputEvent::Button)
        }
        Some(instance_type::OCCUPANCY) => {
            OccupancyEvent::from_info(info).map_or(generic, TypedInputEvent::Occupancy)
        }
        Some(instance_type::ABSOLUTE_INPUT) => {
            TypedInputEvent::Position(MagnitudeEvent::from_info(info))
        }
        Some(instance_type::LIGHT_SENSOR) => {
            TypedInputEvent::Illuminance(MagnitudeEvent::from_info(info))
        }
        _ => generic,
    }
}

pub mod button_filter {
    pub const RELEASE: u8 = 1 << 0;
    pub const PRESS: u8 = 1 << 1;
    pub const SHORT_PRESS: u8 = 1 << 2;
    pub const DOUBLE_PRESS: u8 = 1 << 3;
    pub const LONG_PRESS_START: u8 = 1 << 4;
    pub const LONG_PRESS_REPEAT: u8 = 1 << 5;
    pub const LONG_PRESS_STOP: u8 = 1 << 6;
    pub const STUCK_FREE: u8 = 1 << 7;

    pub const FACTORY_DEFAULT: u8 = SHORT_PRESS
        | LONG_PRESS_START
        | LONG_PRESS_REPEAT
        | LONG_PRESS_STOP
        | STUCK_FREE;
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings, reason = "frame literals group their digits by the fields of a Part 103 frame")]
mod tests {
    use super::*;

    fn event(bytes: [u8; 3]) -> InputEvent {
        decode_event(ForwardFrame24::from_bytes(bytes)).expect("event should decode")
    }

    fn source_of(ev: InputEvent) -> EventSource {
        match ev {
            InputEvent::Instance { source, .. } => source,
            other => panic!("expected an instance event, got {other:?}"),
        }
    }

    #[test]
    fn a_command_frame_is_not_an_event() {
        let frame = ForwardFrame24::from_bytes([0x07, 0xFE, 0x30]);
        assert!(frame.is_command());
        assert_eq!(decode_event(frame), None);
    }

    #[test]
    fn scheme_2_names_the_device_and_the_instance() {
        let src = source_of(event([0x06, 0x80, 0x02]));
        assert_eq!(src.scheme, EventScheme::DeviceInstance);
        assert_eq!(src.short_address, Some(3));
        assert_eq!(src.instance_number, Some(0));
        assert_eq!(src.instance_type, None);
        assert!(src.scheme.identifies_device());
    }

    #[test]
    fn scheme_1_names_the_device_and_the_instance_type() {
        let src = source_of(event([0x06, 0b0_00001_00, 0x02]));
        assert_eq!(src.scheme, EventScheme::Device);
        assert_eq!(src.short_address, Some(3));
        assert_eq!(src.instance_type, Some(instance_type::PUSH_BUTTON));
        assert_eq!(src.instance_number, None);
    }

    #[test]
    fn scheme_0_carries_no_device_identity_and_that_is_legal() {
        let src = source_of(event([0b1_0_00001_0, 0b1_00010_00, 0x02]));
        assert_eq!(src.scheme, EventScheme::Instance);
        assert_eq!(src.short_address, None);
        assert_eq!(src.instance_type, Some(1));
        assert_eq!(src.instance_number, Some(2));
        assert!(
            !src.scheme.identifies_device(),
            "scheme 0 must not claim to identify a device: two identical panels \
             are indistinguishable under it, which is why the product forces 2"
        );
    }

    #[test]
    fn scheme_3_names_a_device_group() {
        let src = source_of(event([0b1_0_00101_0, 0b0_00011_00, 0x00]));
        assert_eq!(src.scheme, EventScheme::DeviceGroup);
        assert_eq!(src.device_group, Some(5));
        assert_eq!(src.instance_type, Some(instance_type::OCCUPANCY));
    }

    #[test]
    fn scheme_4_names_an_instance_group() {
        let src = source_of(event([0b1_1_10000_0, 0b0_00001_00, 0x02]));
        assert_eq!(src.scheme, EventScheme::InstanceGroup);
        assert_eq!(src.instance_group, Some(0b10000));
        assert_eq!(src.instance_type, Some(instance_type::PUSH_BUTTON));
    }

    #[test]
    fn event_info_spans_the_two_low_bits_of_the_middle_byte() {
        match event([0x06, 0x83, 0xFF]) {
            InputEvent::Instance { info, .. } => assert_eq!(info, 0x3FF),
            other => panic!("{other:?}"),
        }
        match event([0x06, 0x80, 0xFF]) {
            InputEvent::Instance { info, .. } => assert_eq!(info, 0x0FF),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn power_notification_is_decoded_before_it_can_look_like_a_scheme() {
        let ev = event([0xFE, 0b111_1_0001, 0b1_000101]);
        match ev {
            InputEvent::PowerCycle {
                short_address,
                device_group,
            } => {
                assert_eq!(short_address, Some(5));
                assert_eq!(device_group, Some(0b00010));
            }
            other => panic!("expected a power cycle, got {other:?}"),
        }
    }

    #[test]
    fn power_notification_without_an_address_reports_none_not_zero() {
        match event([0xFE, 0b111_0_0000, 0b00_000000]) {
            InputEvent::PowerCycle {
                short_address,
                device_group,
            } => {
                assert_eq!(short_address, None);
                assert_eq!(device_group, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_button_event_round_trips_through_its_code() {
        for ev in [
            ButtonEvent::Release,
            ButtonEvent::Press,
            ButtonEvent::ShortPress,
            ButtonEvent::DoublePress,
            ButtonEvent::LongPressStart,
            ButtonEvent::LongPressRepeat,
            ButtonEvent::LongPressStop,
            ButtonEvent::ButtonFree,
            ButtonEvent::ButtonStuck,
        ] {
            assert_eq!(ButtonEvent::from_info(ev.code()), Some(ev), "{ev:?}");
        }
    }

    #[test]
    fn bit_zero_of_every_button_code_is_the_pressed_flag() {
        assert!(!ButtonEvent::Release.is_pressed());
        assert!(ButtonEvent::Press.is_pressed());
        assert!(!ButtonEvent::ShortPress.is_pressed());
        assert!(ButtonEvent::DoublePress.is_pressed());
        assert!(ButtonEvent::LongPressStart.is_pressed());
        assert!(ButtonEvent::LongPressRepeat.is_pressed());
        assert!(!ButtonEvent::LongPressStop.is_pressed());
        assert!(!ButtonEvent::ButtonFree.is_pressed());
        assert!(ButtonEvent::ButtonStuck.is_pressed());
    }

    #[test]
    fn reserved_button_codes_are_ignored_rather_than_rejected() {
        for info in [0x003u16, 0x004, 0x006, 0x007, 0x008, 0x00A, 0x00D, 0x010, 0x3FF] {
            assert_eq!(ButtonEvent::from_info(info), None, "0x{info:03X}");
        }
    }

    #[test]
    fn the_factory_filter_is_0xf4_and_omits_the_delaying_events() {
        assert_eq!(button_filter::FACTORY_DEFAULT, 0xF4);
        assert_eq!(button_filter::FACTORY_DEFAULT & button_filter::DOUBLE_PRESS, 0);
        assert_eq!(button_filter::FACTORY_DEFAULT & button_filter::PRESS, 0);
        assert_eq!(button_filter::FACTORY_DEFAULT & button_filter::RELEASE, 0);
        assert_eq!(button_filter::STUCK_FREE, 0x80);
    }

    #[test]
    fn an_occupancy_event_reports_movement_and_occupancy_together() {
        let ev = OccupancyEvent::from_info(0b0010).expect("occupied, no movement");
        assert!(ev.occupied);
        assert!(!ev.movement);
        assert!(ev.is_transition());

        let both = OccupancyEvent::from_info(0b0011).expect("occupied and movement");
        assert!(both.occupied && both.movement);
    }

    #[test]
    fn the_still_bit_separates_a_transition_from_a_report_timer_repeat() {
        let entered = OccupancyEvent::from_info(0b0011).unwrap();
        let still_there = OccupancyEvent::from_info(0b0111).unwrap();
        assert!(entered.is_transition());
        assert!(!still_there.is_transition());
        assert!(still_there.occupied, "a repeat still carries the state");
    }

    #[test]
    fn bit_three_says_whether_the_sensor_already_applied_a_hold_timer() {
        let presence = OccupancyEvent::from_info(0b0010).unwrap();
        let movement_based = OccupancyEvent::from_info(0b1010).unwrap();
        assert!(!presence.movement_based);
        assert!(movement_based.movement_based);
    }

    #[test]
    fn occupancy_events_round_trip_and_reserved_bits_are_refused() {
        for info in 0u16..16 {
            let ev = OccupancyEvent::from_info(info).expect("bits 0..3 are all defined");
            assert_eq!(ev.to_info(), info);
        }
        assert_eq!(OccupancyEvent::from_info(0b1_0000), None);
        assert_eq!(OccupancyEvent::from_info(0x3FF), None);
    }

    #[test]
    fn the_occupancy_filter_defaults_to_occupied_and_vacant_only() {
        assert_eq!(occupancy_filter::FACTORY_DEFAULT, 0b0000_0011);
        assert_eq!(
            occupancy_filter::FACTORY_DEFAULT & occupancy_filter::MOVEMENT,
            0
        );
    }

    #[test]
    fn magnitude_events_carry_the_raw_field_and_do_not_scale_it() {
        assert_eq!(MagnitudeEvent::from_info(0x3FF).raw, 0x3FF);
        assert_eq!(MagnitudeEvent::from_info(0xFFFF).raw, 0x3FF);
    }

    #[test]
    fn typing_follows_the_instance_type_and_falls_back_honestly() {
        assert_eq!(
            type_event(Some(instance_type::PUSH_BUTTON), 0x002),
            TypedInputEvent::Button(ButtonEvent::ShortPress)
        );
        assert!(matches!(
            type_event(Some(instance_type::OCCUPANCY), 0b0011),
            TypedInputEvent::Occupancy(_)
        ));
        assert!(matches!(
            type_event(Some(instance_type::ABSOLUTE_INPUT), 512),
            TypedInputEvent::Position(_)
        ));
        assert!(matches!(
            type_event(Some(instance_type::LIGHT_SENSOR), 700),
            TypedInputEvent::Illuminance(_)
        ));
        assert_eq!(
            type_event(Some(instance_type::PUSH_BUTTON), 0x003),
            TypedInputEvent::Generic {
                instance_type: Some(1),
                info: 0x003
            }
        );
        assert_eq!(
            type_event(None, 0x002),
            TypedInputEvent::Generic {
                instance_type: None,
                info: 0x002
            }
        );
    }

    #[test]
    fn scheme_codes_match_the_dtr0_values_of_set_event_scheme() {
        for (code, scheme) in [
            (0u8, EventScheme::Instance),
            (1, EventScheme::Device),
            (2, EventScheme::DeviceInstance),
            (3, EventScheme::DeviceGroup),
            (4, EventScheme::InstanceGroup),
        ] {
            assert_eq!(EventScheme::from_code(code), Some(scheme));
            assert_eq!(scheme.code(), code);
        }
        assert_eq!(EventScheme::from_code(5), None);
    }
}
