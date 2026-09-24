use dali2rust_rules_model::{InputRef, LightTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightVerb {
    On { level: Option<u8> },
    Off,
    Level { level: u8 },
    LevelRelative { delta: i16 },
    Dim { delta: i16 },
    Cct { kelvin: u16 },
    CctRelative { delta_k: i32 },
    Xy { x_1e4: u16, y_1e4: u16 },
    Rgb { r: u8, g: u8, b: u8 },
    LastActive,
    StopFade,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Light {
        target: LightTarget,
        verb: LightVerb,
        hold_hcl: bool,
    },
    SceneRecall {
        scene: u8,
        target: Option<LightTarget>,
        hold_hcl: bool,
    },
    SceneApply { scene: u8, hold_hcl: bool },
    HclResume { target: LightTarget },
    HclHold { target: LightTarget },
    HclSchedule { schedule: String, enabled: bool },
    InputFeedback { input: InputRef, on: bool },
    PanelSelect {
        adapter_id: u8,
        group: u8,
        selected: u8,
    },
    CancelHold { input: InputRef },
    CatchMovement { input: InputRef },
    MqttPublish {
        topic: String,
        payload: String,
        retain: bool,
    },
    Log { text: String },
    StatCount { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialReason {
    ConditionUnevaluable,
    EffectBudget,
    ChainDepth,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivationOutcome {
    pub rule: String,
    pub dry: bool,
    pub effects: Vec<Effect>,
    pub partial: Option<PartialReason>,
    pub trigger_kind: &'static str,
}
