use crate::condition::Condition;
use crate::refs::{InputRef, LightTarget};
use crate::time::DurationMs;
use crate::value::{ValueExpr, VarValue};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    LightOn,
    LightOff,
    LightToggle,
    LightLevel,
    LightDim,
    LightDimHold,
    LightCct,
    LightXy,
    LightRgb,
    LightLastActive,
    LightStopFade,
    SceneRecall,
    SceneApply,
    SceneCycle,
    HclResume,
    HclHold,
    HclEnable,
    HclDisable,
    InputFeedbackOn,
    InputFeedbackOff,
    PanelSelect,
    InputCancelHold,
    InputCatchMovement,
    Wait,
    After,
    TimerStart,
    TimerRestart,
    TimerCancel,
    Call,
    Repeat,
    Conditional,
    VarSet,
    VarAdd,
    RuleEnable,
    RuleDisable,
    MqttPublish,
    Log,
    StatCount,
}

pub const ACTION_KIND_COUNT: usize = 38;

const ACTION_KIND_NAMES: [&str; ACTION_KIND_COUNT] = [
    "light_on",
    "light_off",
    "light_toggle",
    "light_level",
    "light_dim",
    "light_dim_hold",
    "light_cct",
    "light_xy",
    "light_rgb",
    "light_last_active",
    "light_stop_fade",
    "scene_recall",
    "scene_apply",
    "scene_cycle",
    "hcl_resume",
    "hcl_hold",
    "hcl_enable",
    "hcl_disable",
    "input_feedback_on",
    "input_feedback_off",
    "panel_select",
    "input_cancel_hold",
    "input_catch_movement",
    "wait",
    "after",
    "timer_start",
    "timer_restart",
    "timer_cancel",
    "call",
    "repeat",
    "conditional",
    "var_set",
    "var_add",
    "rule_enable",
    "rule_disable",
    "mqtt_publish",
    "log",
    "stat_count",
];

impl ActionKind {
    pub const ALL: [ActionKind; ACTION_KIND_COUNT] = [
        ActionKind::LightOn,
        ActionKind::LightOff,
        ActionKind::LightToggle,
        ActionKind::LightLevel,
        ActionKind::LightDim,
        ActionKind::LightDimHold,
        ActionKind::LightCct,
        ActionKind::LightXy,
        ActionKind::LightRgb,
        ActionKind::LightLastActive,
        ActionKind::LightStopFade,
        ActionKind::SceneRecall,
        ActionKind::SceneApply,
        ActionKind::SceneCycle,
        ActionKind::HclResume,
        ActionKind::HclHold,
        ActionKind::HclEnable,
        ActionKind::HclDisable,
        ActionKind::InputFeedbackOn,
        ActionKind::InputFeedbackOff,
        ActionKind::PanelSelect,
        ActionKind::InputCancelHold,
        ActionKind::InputCatchMovement,
        ActionKind::Wait,
        ActionKind::After,
        ActionKind::TimerStart,
        ActionKind::TimerRestart,
        ActionKind::TimerCancel,
        ActionKind::Call,
        ActionKind::Repeat,
        ActionKind::Conditional,
        ActionKind::VarSet,
        ActionKind::VarAdd,
        ActionKind::RuleEnable,
        ActionKind::RuleDisable,
        ActionKind::MqttPublish,
        ActionKind::Log,
        ActionKind::StatCount,
    ];

    pub fn name(self) -> &'static str {
        ACTION_KIND_NAMES[self as usize]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LevelSpec {
    Absolute(ValueExpr),
    Relative { delta: i16 },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CctSpec {
    Absolute(ValueExpr),
    Relative { delta_k: i32 },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum LightOp {
    #[serde(rename = "light_on")]
    On {
        #[serde(skip_serializing_if = "Option::is_none")]
        level: Option<ValueExpr>,
    },
    #[serde(rename = "light_off")]
    Off,
    #[serde(rename = "light_toggle")]
    Toggle {
        #[serde(skip_serializing_if = "Option::is_none")]
        level: Option<ValueExpr>,
    },
    #[serde(rename = "light_level")]
    Level { level: LevelSpec },
    #[serde(rename = "light_dim")]
    Dim { delta: i16 },
    #[serde(rename = "light_dim_hold")]
    DimHold { rate_per_s: i16 },
    #[serde(rename = "light_cct")]
    Cct { cct: CctSpec },
    #[serde(rename = "light_xy")]
    Xy { x_1e4: u16, y_1e4: u16 },
    #[serde(rename = "light_rgb")]
    Rgb { r: u8, g: u8, b: u8 },
    #[serde(rename = "light_last_active")]
    LastActive,
    #[serde(rename = "light_stop_fade")]
    StopFade,
}

impl LightOp {
    fn kind(&self) -> ActionKind {
        match self {
            LightOp::On { .. } => ActionKind::LightOn,
            LightOp::Off => ActionKind::LightOff,
            LightOp::Toggle { .. } => ActionKind::LightToggle,
            LightOp::Level { .. } => ActionKind::LightLevel,
            LightOp::Dim { .. } => ActionKind::LightDim,
            LightOp::DimHold { .. } => ActionKind::LightDimHold,
            LightOp::Cct { .. } => ActionKind::LightCct,
            LightOp::Xy { .. } => ActionKind::LightXy,
            LightOp::Rgb { .. } => ActionKind::LightRgb,
            LightOp::LastActive => ActionKind::LightLastActive,
            LightOp::StopFade => ActionKind::LightStopFade,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LightAction {
    #[serde(flatten)]
    pub op: LightOp,
    pub target: LightTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold_hcl: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum SceneAction {
    #[serde(rename = "scene_recall")]
    Recall {
        scene: ValueExpr,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<LightTarget>,
    },
    #[serde(rename = "scene_apply")]
    Apply { scene: ValueExpr },
    #[serde(rename = "scene_cycle")]
    Cycle { scenes: Vec<u8> },
}

impl SceneAction {
    fn kind(&self) -> ActionKind {
        match self {
            SceneAction::Recall { .. } => ActionKind::SceneRecall,
            SceneAction::Apply { .. } => ActionKind::SceneApply,
            SceneAction::Cycle { .. } => ActionKind::SceneCycle,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum HclAction {
    #[serde(rename = "hcl_resume")]
    Resume { target: LightTarget },
    #[serde(rename = "hcl_hold")]
    Hold { target: LightTarget },
    #[serde(rename = "hcl_enable")]
    Enable { schedule: String },
    #[serde(rename = "hcl_disable")]
    Disable { schedule: String },
}

impl HclAction {
    fn kind(&self) -> ActionKind {
        match self {
            HclAction::Resume { .. } => ActionKind::HclResume,
            HclAction::Hold { .. } => ActionKind::HclHold,
            HclAction::Enable { .. } => ActionKind::HclEnable,
            HclAction::Disable { .. } => ActionKind::HclDisable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum InputAction {
    #[serde(rename = "input_feedback_on")]
    FeedbackOn { input: InputRef },
    #[serde(rename = "input_feedback_off")]
    FeedbackOff { input: InputRef },
    #[serde(rename = "panel_select")]
    PanelSelect {
        adapter_id: u8,
        group: u8,
        selected: ValueExpr,
    },
    #[serde(rename = "input_cancel_hold")]
    CancelHold { input: InputRef },
    #[serde(rename = "input_catch_movement")]
    CatchMovement { input: InputRef },
}

impl InputAction {
    fn kind(&self) -> ActionKind {
        match self {
            InputAction::FeedbackOn { .. } => ActionKind::InputFeedbackOn,
            InputAction::FeedbackOff { .. } => ActionKind::InputFeedbackOff,
            InputAction::PanelSelect { .. } => ActionKind::PanelSelect,
            InputAction::CancelHold { .. } => ActionKind::InputCancelHold,
            InputAction::CatchMovement { .. } => ActionKind::InputCatchMovement,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum FlowAction {
    #[serde(rename = "wait")]
    Wait { duration_ms: DurationMs },
    #[serde(rename = "after")]
    After {
        delay_ms: DurationMs,
        actions: Vec<Action>,
    },
    #[serde(rename = "timer_start")]
    TimerStart { timer: String, duration_ms: DurationMs },
    #[serde(rename = "timer_restart")]
    TimerRestart { timer: String, duration_ms: DurationMs },
    #[serde(rename = "timer_cancel")]
    TimerCancel { timer: String },
    #[serde(rename = "call")]
    Call { block: String },
    #[serde(rename = "repeat")]
    Repeat { count: u8, actions: Vec<Action> },
    #[serde(rename = "conditional")]
    Conditional {
        condition: Condition,
        then_actions: Vec<Action>,
        else_actions: Vec<Action>,
    },
}

impl FlowAction {
    fn kind(&self) -> ActionKind {
        match self {
            FlowAction::Wait { .. } => ActionKind::Wait,
            FlowAction::After { .. } => ActionKind::After,
            FlowAction::TimerStart { .. } => ActionKind::TimerStart,
            FlowAction::TimerRestart { .. } => ActionKind::TimerRestart,
            FlowAction::TimerCancel { .. } => ActionKind::TimerCancel,
            FlowAction::Call { .. } => ActionKind::Call,
            FlowAction::Repeat { .. } => ActionKind::Repeat,
            FlowAction::Conditional { .. } => ActionKind::Conditional,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum StateAction {
    #[serde(rename = "var_set")]
    VarSet { name: String, value: VarValue },
    #[serde(rename = "var_add")]
    VarAdd { name: String, delta: ValueExpr },
    #[serde(rename = "rule_enable")]
    RuleEnable { rule: String },
    #[serde(rename = "rule_disable")]
    RuleDisable { rule: String },
    #[serde(rename = "mqtt_publish")]
    MqttPublish {
        topic: String,
        payload: String,
        retain: bool,
    },
    #[serde(rename = "log")]
    Log { text: String },
    #[serde(rename = "stat_count")]
    StatCount { name: String },
}

impl StateAction {
    fn kind(&self) -> ActionKind {
        match self {
            StateAction::VarSet { .. } => ActionKind::VarSet,
            StateAction::VarAdd { .. } => ActionKind::VarAdd,
            StateAction::RuleEnable { .. } => ActionKind::RuleEnable,
            StateAction::RuleDisable { .. } => ActionKind::RuleDisable,
            StateAction::MqttPublish { .. } => ActionKind::MqttPublish,
            StateAction::Log { .. } => ActionKind::Log,
            StateAction::StatCount { .. } => ActionKind::StatCount,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Action {
    Light(LightAction),
    Scene(SceneAction),
    Hcl(HclAction),
    Input(InputAction),
    Flow(FlowAction),
    State(StateAction),
}

impl Action {
    pub fn kind(&self) -> ActionKind {
        match self {
            Action::Light(a) => a.op.kind(),
            Action::Scene(a) => a.kind(),
            Action::Hcl(a) => a.kind(),
            Action::Input(a) => a.kind(),
            Action::Flow(a) => a.kind(),
            Action::State(a) => a.kind(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_kind_names_align_with_discriminants() {
        for (i, kind) in ActionKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, i, "ALL order must match discriminants");
            assert_eq!(kind.name(), ACTION_KIND_NAMES[i]);
        }
    }
}
