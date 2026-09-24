use crate::error::CompileError;
use crate::refs::{DeviceRef, GroupRef, InputDeviceRef, LampRef};
use crate::rule::RuleSet;

pub trait NameResolver: Send + Sync {
    fn primary_adapter(&self) -> u8;

    fn adapter_exists(&self, adapter_id: u8) -> bool;

    fn resolve_lamp(&self, name: &str) -> Option<LampRef>;

    fn resolve_group(&self, name: &str) -> Option<GroupRef>;

    fn resolve_device(&self, name: &str) -> Option<DeviceRef>;

    fn resolve_input_device(&self, name: &str) -> Option<InputDeviceRef>;

    fn resolve_scene(&self, name: &str) -> Option<u8>;
}

pub trait RuleCompiler: Send + Sync {
    fn lang_id(&self) -> u8;

    fn compile(&self, source: &str, resolver: &dyn NameResolver) -> Result<RuleSet, CompileError>;
}
