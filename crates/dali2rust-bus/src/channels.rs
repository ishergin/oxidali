pub trait Sender<T> {
    fn try_send(&self, item: T) -> Result<(), T>;
}

pub trait Receiver<T> {
    fn recv(&self) -> Result<T, RecvError>;

    fn recv_timeout(&self, timeout_ms: u32) -> Result<T, RecvTimeoutError>;

    fn try_recv(&self) -> Result<T, TryRecvError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvError {
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvTimeoutError {
    Timeout,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    Empty,
    Disconnected,
}
