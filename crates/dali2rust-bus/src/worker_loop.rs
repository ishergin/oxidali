use std::time::Duration;

use crate::{BusFrame, BusSubscriberRx};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerTurn {
    Handled,
    Idle,
    Disconnected,
}

pub fn recv_then_drain(
    rx: &BusSubscriberRx,
    timeout: Duration,
    mut on_frame: impl FnMut(BusFrame),
) -> WorkerTurn {
    match rx.recv_timeout(timeout) {
        Ok(frame) => {
            on_frame(frame);
            while let Ok(frame) = rx.try_recv() {
                on_frame(frame);
            }
            WorkerTurn::Handled
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => WorkerTurn::Idle,
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => WorkerTurn::Disconnected,
    }
}
