use std::sync::Arc;

use dali2rust_platform::dali::WireLease;

use crate::dali::commands::{DaliCommand, DaliResponse};
use crate::dali::frame::{ForwardFrame, HoldCause};
use crate::dali::ses::{DaliSession, TransactionPriority};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame24Fault {
    Unsupported,
    Contended(Frame24Contention),
    Transport,
    Preempted,
    Held(HoldCause),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame24Contention {
    Collision,
    BusBusy,
    ForeignInWindow,
    ContendedNoAnswer,
    PairSplit,
}

impl Frame24Contention {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Collision => "frame24_collision",
            Self::BusBusy => "frame24_bus_busy",
            Self::ForeignInWindow => "frame24_foreign_in_window",
            Self::ContendedNoAnswer => "frame24_contended_no_answer",
            Self::PairSplit => "frame24_pair_split",
        }
    }
}

pub trait WireHold: Send + Sync {
    fn held(&self) -> Option<HoldCause>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadbackWorkaround {
    GroupsCorrected,
    ColourFeaturesCorrected,
    ExtendedFadeCorrected,
    ProgramRepair,
}

pub trait DaliProductController {
    type Error: core::fmt::Debug;

    fn send_command(&mut self, cmd: &DaliCommand) -> Result<DaliResponse, Self::Error>;

    fn send_command_observed(
        &mut self,
        cmd: &DaliCommand,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        Ok((self.send_command(cmd)?, false))
    }

    fn session(&self) -> &DaliSession;
}

pub trait DaliApplicationController: DaliProductController {
    fn send_raw(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<DaliResponse, Self::Error>;

    fn send_raw_observed(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        Ok((self.send_raw(frame, expects_backward)?, false))
    }

    fn send_frame24(
        &mut self,
        _frame: [u8; 3],
        _expects_backward: bool,
    ) -> Result<DaliResponse, Frame24Fault> {
        Err(Frame24Fault::Unsupported)
    }

    fn send_frame24_once(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
    ) -> Result<DaliResponse, Frame24Fault> {
        self.send_frame24(frame, expects_backward)
    }

    // IEC 62386-101 §9.3
    fn send_frame24_twice(&mut self, frame: [u8; 3]) -> Result<(), Frame24Fault> {
        self.send_frame24(frame, false)?;
        self.send_frame24(frame, false).map(|_| ())
    }

    fn supports_frame24(&self) -> bool {
        false
    }

    fn send_raw_once(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        Ok((self.send_raw(frame, expects_backward)?, false))
    }

    fn with_wire_lease<R>(&mut self, lease: WireLease, run: impl FnOnce(&mut Self) -> R) -> R {
        let _ = lease;
        run(self)
    }

    fn with_wire_hold<R>(
        &mut self,
        hold: Arc<dyn WireHold>,
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let _ = hold;
        run(self)
    }

    fn step_boundary(&mut self) {}

    fn step<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        self.step_boundary();
        run(self)
    }

    fn with_wire_class<R>(
        &mut self,
        class: Option<TransactionPriority>,
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let _ = class;
        run(self)
    }

    // IEC 62386-101 §9.2
    fn transaction<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        run(self)
    }

    // IEC 62386-101 §9.1.4, §9.2
    fn unit<R>(&mut self, mut run: impl FnMut(&mut Self) -> R) -> R {
        self.transaction(|c| run(c))
    }

    fn unit_exempt<R>(&mut self, mut run: impl FnMut(&mut Self) -> R) -> R {
        self.transaction_exempt(|c| run(c))
    }

    fn send_raw_pair(
        &mut self,
        frame: ForwardFrame,
        expects_backward: bool,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        let (_, first_contended) = self.send_raw_observed(frame, expects_backward)?;
        let (response, contended) = self.send_raw_observed(frame, expects_backward)?;
        Ok((response, first_contended || contended))
    }

    fn send_raw_enabled_query(
        &mut self,
        enable: ForwardFrame,
        frame: ForwardFrame,
    ) -> Result<(DaliResponse, bool), Self::Error> {
        let (_, enable_contended) = self.send_raw_observed(enable, false)?;
        let (response, contended) = self.send_raw_once(frame, true)?;
        Ok((response, enable_contended || contended))
    }

    fn transaction_exempt<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        run(self)
    }

    fn note_workaround(&mut self, workaround: ReadbackWorkaround) {
        let _ = workaround;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_CONTENTION: [(Frame24Contention, &str); 5] = [
        (Frame24Contention::Collision, "frame24_collision"),
        (Frame24Contention::BusBusy, "frame24_bus_busy"),
        (Frame24Contention::ForeignInWindow, "frame24_foreign_in_window"),
        (Frame24Contention::ContendedNoAnswer, "frame24_contended_no_answer"),
        (Frame24Contention::PairSplit, "frame24_pair_split"),
    ];

    #[test]
    fn every_wire_fact_behind_a_contention_has_its_own_word() {
        for (cause, message) in EVERY_CONTENTION {
            assert_eq!(cause.message(), message);
        }
        let mut words: Vec<&str> = EVERY_CONTENTION.iter().map(|(_, m)| *m).collect();
        words.sort_unstable();
        words.dedup();
        assert_eq!(
            words.len(),
            EVERY_CONTENTION.len(),
            "two wire facts under one word leave the operator where this started"
        );
    }

    #[test]
    fn a_contended_fault_carries_its_cause_and_a_transport_fault_is_not_one() {
        assert_ne!(
            Frame24Fault::Contended(Frame24Contention::Collision),
            Frame24Fault::Contended(Frame24Contention::BusBusy)
        );
        assert_ne!(
            Frame24Fault::Transport,
            Frame24Fault::Contended(Frame24Contention::Collision)
        );
    }
}
