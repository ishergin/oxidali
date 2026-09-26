use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use crate::halfbits::HalfBitBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExchangeId(pub u32);

const CMD_IDLE: u8 = 0;
const CMD_START_TX: u8 = 1;

pub struct AtomicCommandCell {
    state: AtomicU8,
    tx_data: [AtomicU8; HalfBitBuffer::DATA_LEN],
    tx_halfbit_len: AtomicU8,
    expects_backward: AtomicBool,
    min_idle_ticks: AtomicU8,
    restart_gate: AtomicU8,
    exchange_id: AtomicU32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxGates {
    pub min_idle_ticks: u8,
    pub restart_gate: u8,
}

impl TxGates {
    pub const fn settle(min_idle_ticks: u8) -> Self {
        Self {
            min_idle_ticks,
            restart_gate: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxCommand {
    pub data: [u8; HalfBitBuffer::DATA_LEN],
    pub len: u8,
    pub expects_backward: bool,
    pub min_idle_ticks: u8,
    pub restart_gate: u8,
    pub exchange_id: ExchangeId,
}

impl AtomicCommandCell {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(CMD_IDLE),
            tx_data: [const { AtomicU8::new(0) }; HalfBitBuffer::DATA_LEN],
            tx_halfbit_len: AtomicU8::new(0),
            expects_backward: AtomicBool::new(false),
            min_idle_ticks: AtomicU8::new(0),
            restart_gate: AtomicU8::new(0),
            exchange_id: AtomicU32::new(0),
        }
    }

    pub fn send_packed_tx(
        &self,
        halfbit_data: &[u8; HalfBitBuffer::DATA_LEN],
        halfbit_len: u8,
        expects_backward: bool,
        gates: TxGates,
        exchange_id: ExchangeId,
    ) -> bool {
        if halfbit_len == 0 || halfbit_len > HalfBitBuffer::MAX_DATA_HALF_BITS {
            return false;
        }

        if self.state.load(Ordering::Acquire) != CMD_IDLE {
            return false;
        }

        for (cell, &b) in self.tx_data.iter().zip(halfbit_data.iter()) {
            cell.store(b, Ordering::Relaxed);
        }
        self.expects_backward
            .store(expects_backward, Ordering::Relaxed);
        self.min_idle_ticks.store(gates.min_idle_ticks, Ordering::Relaxed);
        self.restart_gate.store(gates.restart_gate, Ordering::Relaxed);
        self.exchange_id.store(exchange_id.0, Ordering::Relaxed);
        self.tx_halfbit_len.store(halfbit_len, Ordering::Relaxed);

        self.state
            .compare_exchange(CMD_IDLE, CMD_START_TX, Ordering::Release, Ordering::Relaxed)
            .is_ok()
    }

    #[cfg_attr(target_os = "espidf", link_section = ".iram1.dali_phy")]
    pub fn take_command(&self) -> Option<TxCommand> {
        if self.state.load(Ordering::Acquire) != CMD_START_TX {
            return None;
        }
        let len = self.tx_halfbit_len.load(Ordering::Relaxed);
        const _: () = assert!(
            HalfBitBuffer::DATA_LEN == 9,
            "the array pattern below spells out every element"
        );
        let [b0, b1, b2, b3, b4, b5, b6, b7, b8] = &self.tx_data;
        let data = [
            b0.load(Ordering::Relaxed),
            b1.load(Ordering::Relaxed),
            b2.load(Ordering::Relaxed),
            b3.load(Ordering::Relaxed),
            b4.load(Ordering::Relaxed),
            b5.load(Ordering::Relaxed),
            b6.load(Ordering::Relaxed),
            b7.load(Ordering::Relaxed),
            b8.load(Ordering::Relaxed),
        ];
        let expects_backward = self.expects_backward.load(Ordering::Relaxed);
        let min_idle_ticks = self.min_idle_ticks.load(Ordering::Relaxed);
        let restart_gate = self.restart_gate.load(Ordering::Relaxed);
        let exchange_id = ExchangeId(self.exchange_id.load(Ordering::Relaxed));
        self.state.store(CMD_IDLE, Ordering::Release);
        Some(TxCommand {
            data,
            len,
            expects_backward,
            min_idle_ticks,
            restart_gate,
            exchange_id,
        })
    }
}

impl Default for AtomicCommandCell {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_take_roundtrip() {
        let cell = AtomicCommandCell::new();

        let mut data = [0u8; 9];
        data[0] = 0x55;
        data[1] = 0xAA;
        data[2] = 0x01;
        assert!(cell.send_packed_tx(&data, 24, false, TxGates::settle(0), ExchangeId(1)));

        let taken = cell.take_command().unwrap();
        assert_eq!(taken.data[0], 0x55);
        assert_eq!(taken.data[1], 0xAA);
        assert_eq!(taken.data[2], 0x01);
        assert_eq!(taken.len, 24);
        assert!(!taken.expects_backward);

        assert!(cell.take_command().is_none());
    }

    #[test]
    fn send_with_backward_expectation_roundtrip() {
        let cell = AtomicCommandCell::new();
        let mut data = [0u8; 9];
        data[0] = 0x12;
        data[1] = 0x34;

        assert!(cell.send_packed_tx(&data, 16, true, TxGates::settle(0), ExchangeId(2)));

        let taken = cell.take_command().unwrap();
        assert_eq!(taken.data[0], 0x12);
        assert_eq!(taken.data[1], 0x34);
        assert_eq!(taken.len, 16);
        assert!(taken.expects_backward);
        assert_eq!(taken.exchange_id, ExchangeId(2));
    }

    #[test]
    fn packed_halfbit_length_can_exceed_packed_byte_count() {
        let cell = AtomicCommandCell::new();
        let data = [0xFF; 9];

        assert!(cell.send_packed_tx(&data, 38, true, TxGates::settle(0), ExchangeId(3)));

        let taken = cell.take_command().unwrap();
        assert_eq!(taken.data, data);
        assert_eq!(taken.len, 38);
        assert!(taken.expects_backward);
    }

    #[test]
    fn every_payload_byte_survives_the_cell() {
        let cell = AtomicCommandCell::new();
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8, 9];

        assert!(cell.send_packed_tx(&data, 72, false, TxGates::settle(0), ExchangeId(4)));
        assert_eq!(cell.take_command().unwrap().data, data);
    }

    #[test]
    fn rejects_invalid_halfbit_lengths() {
        let cell = AtomicCommandCell::new();
        let data = [0xFF; 9];

        assert!(!cell.send_packed_tx(&data, 0, false, TxGates::settle(0), ExchangeId(5)));
        assert!(!cell.send_packed_tx(&data, 73, false, TxGates::settle(0), ExchangeId(5)));
    }

    #[test]
    fn no_double_send() {
        let cell = AtomicCommandCell::new();
        let mut first = [0u8; 9];
        first[0] = 0x01;
        let mut second = [0u8; 9];
        second[0] = 0x02;
        assert!(cell.send_packed_tx(&first, 8, false, TxGates::settle(0), ExchangeId(6)));
        assert!(!cell.send_packed_tx(&second, 8, false, TxGates::settle(0), ExchangeId(7)));
    }

    #[test]
    fn concurrent_takes_never_yield_hybrid_or_stale_frames() {
        use std::{sync::{atomic::AtomicBool as StopFlag, Arc}, thread};

        let frame_a = ([0x11u8; 9], 16u8, false, 7u8);
        let frame_b = ([0xEEu8; 9], 38u8, true, 193u8);
        let cell = Arc::new(AtomicCommandCell::new());
        let stop = Arc::new(StopFlag::new(false));

        let taker = {
            let cell = Arc::clone(&cell);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let mut taken = 0u32;
                while !stop.load(Ordering::Acquire) {
                    if let Some(cmd) = cell.take_command() {
                        let key = (cmd.data, cmd.len, cmd.expects_backward, cmd.min_idle_ticks);
                        assert!(
                            key == frame_a || key == frame_b,
                            "hybrid/stale frame escaped the cell: {cmd:?}"
                        );
                        taken += 1;
                    }
                }
                taken
            })
        };

        const SENDS: u32 = 100_000;
        let mut sent = 0u32;
        while sent < SENDS {
            let (data, len, exp, min_idle_ticks) = if sent % 2 == 0 { frame_a } else { frame_b };
            if cell.send_packed_tx(&data, len, exp, TxGates::settle(min_idle_ticks), ExchangeId(sent + 1)) {
                sent += 1;
            }
        }
        stop.store(true, Ordering::Release);
        let taken = taker.join().unwrap();
        let tail = u32::from(cell.take_command().is_some());
        assert_eq!(
            taken + tail,
            SENDS,
            "every accepted send is taken exactly once"
        );
    }

    #[test]
    fn concurrent_send_take() {
        use std::sync::Arc;
        use std::thread;

        let cell = Arc::new(AtomicCommandCell::new());
        let cell_clone = Arc::clone(&cell);

        let handle = thread::spawn(move || {
            let mut data = [0u8; 9];
            data[0] = 0xAA;
            while !cell_clone.send_packed_tx(&data, 8, false, TxGates::settle(0), ExchangeId(8)) {
                thread::yield_now();
            }
        });

        handle.join().unwrap();
        let taken = cell.take_command().unwrap();
        assert_eq!(taken.data[0], 0xAA);
        assert!(cell.take_command().is_none());
    }
}
