use super::world::HclTargetKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventKind {
    Press,
    Release,
    ShortPress,
    DoublePress,
    LongPressStart,
    LongPressRepeat,
    LongPressStop,
    ButtonStuck,
    ButtonFree,
    BecameOccupied,
    BecameVacant,
    Movement,
    NoMovement,
    LightCrossedAbove,
    LightCrossedBelow,
    PositionChanged,
    Generic,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EngineInput<'a> {
    InputEvent {
        adapter_id: u8,
        short_address: Option<u8>,
        instance_number: Option<u8>,
        instance_type: Option<u8>,
        instance_groups: [Option<u8>; 3],
        kind: InputEventKind,
        value: u16,
    },
    LampChanged {
        adapter_id: u8,
        lamp_id: u16,
        is_on: bool,
        level: u8,
        was_on: bool,
        previous_level: u8,
    },
    GroupChanged {
        adapter_id: u8,
        group_id: u16,
        any_on: bool,
        was_any_on: bool,
    },
    SceneRecalled {
        adapter_id: u8,
        scene_id: u8,
    },
    DevicePresence {
        adapter_id: u8,
        short_address: u8,
        online: bool,
    },
    HclOverride {
        started: bool,
        target: HclTargetKey,
    },
    ControllerActive {
        active: bool,
    },
    ControllerStarts,
    PowerCycled {
        adapter_id: u8,
        short_address: Option<u8>,
    },
    ManualConfigChanged {
        adapter_id: u8,
        short_address: u8,
    },
    RunRule {
        name: &'a str,
        dry: bool,
    },
    Tick,
    RuleFailed {
        name: &'a str,
    },
}
