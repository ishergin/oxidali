pub mod cursor;
pub mod lexer;
pub mod parser;
pub mod testing;

use dali2rust_rules_model::limits::MAX_RULES_SOURCE_BYTES;
use dali2rust_rules_model::{validate, CompileError, NameResolver, RuleCompiler, RuleSet};

pub const LANG_RULES_V1: u8 = 1;

pub use parser::action::PARSED_ACTION_KINDS;
pub use parser::condition::PARSED_CONDITION_KINDS;
pub use parser::expr::PARSED_VALUE_KINDS;
pub use parser::trigger::PARSED_TRIGGER_KINDS;

#[derive(Debug, Default, Clone, Copy)]
pub struct RulesLangV1;

impl RuleCompiler for RulesLangV1 {
    fn lang_id(&self) -> u8 {
        LANG_RULES_V1
    }

    fn compile(&self, source: &str, resolver: &dyn NameResolver) -> Result<RuleSet, CompileError> {
        if source.len() > MAX_RULES_SOURCE_BYTES {
            return Err(CompileError::at(
                1,
                1,
                format!(
                    "source is {} bytes, the stored-document limit is {MAX_RULES_SOURCE_BYTES}",
                    source.len()
                ),
            ));
        }
        let (set, spans) = parser::parse_document(source, resolver, LANG_RULES_V1)?;
        validate(&set).map_err(|e| spans.locate(&e))?;
        Ok(set)
    }
}
