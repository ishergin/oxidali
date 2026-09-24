use dali2rust_platform::dali::WireLease;

use crate::dali::commands::{DaliCommand, DaliResponse};
use crate::dali::frame::ForwardFrame;
use crate::dali::ses::{DaliSession, TransactionPriority};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame24Fault {
    Unsupported,
    Contended,
    Preempted,
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

    fn step_boundary(&mut self) {}

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
}
