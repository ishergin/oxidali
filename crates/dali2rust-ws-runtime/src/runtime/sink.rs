use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsSinkError {
    Closed,
    Failed,
}

pub trait WsSink: Send + Sync {
    fn send_batch(&self, frames: &[Arc<str>]) -> Result<(), WsSinkError>;
    fn close(&self);
}
