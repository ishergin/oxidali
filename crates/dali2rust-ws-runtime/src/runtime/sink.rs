#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsSinkError {
    Closed,
    Failed,
}

pub trait WsSink: Send + Sync {
    fn send_text(&self, text: &str) -> Result<(), WsSinkError>;
    fn close(&self);
}
