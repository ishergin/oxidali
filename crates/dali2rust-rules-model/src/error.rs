use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl CompileError {
    pub fn at(line: u32, column: u32, message: impl Into<String>) -> CompileError {
        CompileError {
            line,
            column,
            message: message.into(),
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    TooManyRules { count: usize },
    TooManyBlocks { count: usize },
    DuplicateRuleName { name: String },
    DuplicateBlockName { name: String },
    NameTooLong { name: String },
    NoTriggers { rule: String },
    TooManyTriggers { rule: String, count: usize },
    TooManyConditions { rule: String, count: usize },
    NoActions { owner: String },
    TooManyActions { rule: String, count: usize },
    TooManyBlockActions { block: String, count: usize },
    UnresolvedBlock { block: String, referenced_by: Vec<String> },
    UnknownRuleReference { rule: String, referenced_by: String },
    CallDepthExceeded { rule: String, block: String },
    NestedRepeat { rule: String },
    NestedConditional { rule: String },
    NestedAfter { rule: String },
    ExpandedActionsExceeded { rule: String, count: usize },
    RepeatCountOutOfRange { rule: String, count: u8 },
    EveryPeriodTooShort { rule: String, period_ms: u32 },
    VarTextTooLong { rule: String, text: String },
    MqttTopicTooLong { rule: String, bytes: usize },
    MqttPayloadTooLong { rule: String, bytes: usize },
    SceneCycleTooLong { rule: String, count: usize },
    SceneOutOfRange { rule: String, scene: u8 },
}

impl ModelError {
    pub fn anchor(&self) -> Option<&str> {
        match self {
            ModelError::TooManyRules { .. } | ModelError::TooManyBlocks { .. } => None,
            ModelError::DuplicateRuleName { name }
            | ModelError::DuplicateBlockName { name }
            | ModelError::NameTooLong { name } => Some(name),
            ModelError::NoTriggers { rule }
            | ModelError::TooManyTriggers { rule, .. }
            | ModelError::TooManyConditions { rule, .. }
            | ModelError::TooManyActions { rule, .. }
            | ModelError::UnknownRuleReference { referenced_by: rule, .. }
            | ModelError::CallDepthExceeded { rule, .. }
            | ModelError::NestedRepeat { rule }
            | ModelError::NestedConditional { rule }
            | ModelError::NestedAfter { rule }
            | ModelError::ExpandedActionsExceeded { rule, .. }
            | ModelError::RepeatCountOutOfRange { rule, .. }
            | ModelError::EveryPeriodTooShort { rule, .. }
            | ModelError::VarTextTooLong { rule, .. }
            | ModelError::MqttTopicTooLong { rule, .. }
            | ModelError::MqttPayloadTooLong { rule, .. }
            | ModelError::SceneCycleTooLong { rule, .. }
            | ModelError::SceneOutOfRange { rule, .. } => Some(rule),
            ModelError::NoActions { owner } => Some(owner),
            ModelError::TooManyBlockActions { block, .. } => Some(block),
            ModelError::UnresolvedBlock { referenced_by, .. } => {
                referenced_by.first().map(String::as_str)
            }
        }
    }
}

fn quote_list(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    quoted.join(", ")
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ModelError as E;
        match self {
            E::TooManyRules { count } => write!(f, "too many rules: {count}"),
            E::TooManyBlocks { count } => write!(f, "too many def blocks: {count}"),
            E::DuplicateRuleName { name } => write!(f, "duplicate rule name \"{name}\""),
            E::DuplicateBlockName { name } => write!(f, "duplicate block name \"{name}\""),
            E::NameTooLong { name } => write!(f, "name too long: \"{name}\""),
            E::NoTriggers { rule } => write!(f, "rule \"{rule}\" has no `when` trigger"),
            E::TooManyTriggers { rule, count } => write!(f, "rule \"{rule}\" has {count} triggers"),
            E::TooManyConditions { rule, count } => write!(f, "rule \"{rule}\" has {count} conditions"),
            E::NoActions { owner } => write!(f, "\"{owner}\" has no actions"),
            E::TooManyActions { rule, count } => write!(f, "rule \"{rule}\" has {count} actions"),
            E::TooManyBlockActions { block, count } => write!(f, "block \"{block}\" has {count} actions"),
            E::UnresolvedBlock { block, referenced_by } => write!(f, "unknown block \"{block}\" referenced by rules: {}", quote_list(referenced_by)),
            E::UnknownRuleReference { rule, referenced_by } => write!(f, "unknown rule \"{rule}\" referenced by rule \"{referenced_by}\""),
            E::CallDepthExceeded { rule, block } => write!(f, "rule \"{rule}\": call of block \"{block}\" exceeds call depth"),
            E::NestedRepeat { rule } => write!(f, "rule \"{rule}\": nested repeat"),
            E::NestedConditional { rule } => write!(f, "rule \"{rule}\": nested if/else"),
            E::NestedAfter { rule } => write!(f, "rule \"{rule}\": nested after block"),
            E::ExpandedActionsExceeded { rule, count } => write!(f, "rule \"{rule}\" expands to {count} actions"),
            E::RepeatCountOutOfRange { rule, count } => write!(f, "rule \"{rule}\": repeat count {count} out of range"),
            E::EveryPeriodTooShort { rule, period_ms } => write!(f, "rule \"{rule}\": every period {period_ms} ms below 1 s"),
            E::VarTextTooLong { rule, text } => write!(f, "rule \"{rule}\": var text too long: \"{text}\""),
            E::MqttTopicTooLong { rule, bytes } => write!(f, "rule \"{rule}\": mqtt topic is {bytes} bytes"),
            E::MqttPayloadTooLong { rule, bytes } => write!(f, "rule \"{rule}\": mqtt payload is {bytes} bytes"),
            E::SceneCycleTooLong { rule, count } => write!(f, "rule \"{rule}\": scene.cycle lists {count} scenes"),
            E::SceneOutOfRange { rule, scene } => write!(f, "rule \"{rule}\": scene {scene} out of range"),
        }
    }
}
