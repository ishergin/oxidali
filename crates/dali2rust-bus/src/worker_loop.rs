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
    on_frame: impl FnMut(BusFrame),
) -> WorkerTurn {
    match rx.recv_timeout(timeout) {
        Ok(frame) => drain_after(rx, frame, on_frame),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => WorkerTurn::Idle,
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => WorkerTurn::Disconnected,
    }
}

pub fn wait_then_drain(rx: &BusSubscriberRx, on_frame: impl FnMut(BusFrame)) -> WorkerTurn {
    match rx.recv() {
        Ok(frame) => drain_after(rx, frame, on_frame),
        Err(std::sync::mpsc::RecvError) => WorkerTurn::Disconnected,
    }
}

fn drain_after(rx: &BusSubscriberRx, first: BusFrame, mut on_frame: impl FnMut(BusFrame)) -> WorkerTurn {
    on_frame(first);
    while let Ok(frame) = rx.try_recv() {
        on_frame(frame);
    }
    WorkerTurn::Handled
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use dali2rust_contracts::bus::command_envelope;
    use dali2rust_contracts::msg::{BusCommandPayload, DaliCommandPayload};

    use super::*;

    fn frame(command: u8) -> BusFrame {
        BusFrame::command(command_envelope(0, 1, 0, None, DaliCommandPayload {
            wire_address: 1,
            command,
            repeat_count: 1,
            raw_mode: false,
            raw_expects_backward: false,
        }))
    }

    fn command_of(frame: &BusFrame) -> u8 {
        let BusFrame::Command(env) = frame else {
            panic!("a command frame");
        };
        let BusCommandPayload::DaliCommandPayload(body) = &env.payload else {
            panic!("a DALI command payload");
        };
        body.command
    }

    #[test]
    fn a_wait_hands_over_every_queued_frame_in_one_turn() {
        let (tx, rx) = mpsc::channel();
        tx.send(frame(1)).unwrap();
        tx.send(frame(2)).unwrap();
        let mut seen = Vec::new();
        let turn = wait_then_drain(&rx, |frame| seen.push(command_of(&frame)));
        assert_eq!(turn, WorkerTurn::Handled);
        assert_eq!(seen, [1, 2]);
    }

    #[test]
    fn a_wait_ends_when_the_bus_is_gone() {
        let (tx, rx) = mpsc::channel::<BusFrame>();
        drop(tx);
        let turn = wait_then_drain(&rx, |_| panic!("no frame was sent"));
        assert_eq!(turn, WorkerTurn::Disconnected);
    }
}
