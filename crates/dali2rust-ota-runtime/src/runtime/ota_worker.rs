use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{
    publish_required, BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx,
    REQUIRED_PUBLISH_BACKOFF_MS, REQUIRED_PUBLISH_UNCAPPED,
};
use dali2rust_contracts::bus::event_envelope;
use dali2rust_contracts::msg::commands::FirmwareUpdateBeginCommand;
use dali2rust_contracts::msg::{ErrorCode, OperationWorkerSignalEvent, Origin};
use dali2rust_platform::firmware::{
    FirmwareError, FirmwareImageSource, FirmwareSink, FirmwareUpdatePort, MaintenanceHold,
};

use crate::runtime::state::{OtaPhase, OtaState};

const SOURCE_ID_UNSPECIFIED: u16 = 0;

const REBOOT_GRACE: Duration = Duration::from_secs(3);

pub const OTA_WORKER_REQUIRED_EVENTS: &[&str] = &["OperationWorkerSignalEvent"];

pub struct OtaWorker {
    publisher: BusPublisher,
    bus_id: BusId,
    port: Arc<dyn FirmwareUpdatePort>,
    source: Arc<dyn FirmwareImageSource>,
    state: Arc<OtaState>,
    hold: Arc<MaintenanceHold>,
}

dali2rust_contracts::dispatch_bus_commands! {
    pub const OTA_WORKER_HANDLED_COMMANDS;
    fn dispatch_ota_command(
        payload: &dali2rust_contracts::msg::BusCommandPayload,
        worker: &mut OtaWorker,
        correlation_id: u64,
    );
    payload = payload;
    ignored = {};
    FirmwareUpdateBeginCommand(begin) => worker.on_begin(correlation_id, begin),
}

struct SlotSink<'a> {
    port: &'a dyn FirmwareUpdatePort,
    state: &'a OtaState,
    opened: bool,
}

impl SlotSink<'_> {
    fn open(&mut self, total: Option<u32>) -> Result<(), FirmwareError> {
        if self.opened {
            return Ok(());
        }
        self.port.begin(total)?;
        self.opened = true;
        Ok(())
    }
}

impl FirmwareSink for SlotSink<'_> {
    fn total(&mut self, bytes: Option<u32>) -> Result<(), FirmwareError> {
        if let Some(bytes) = bytes {
            self.state.set_total(bytes);
        }
        self.open(bytes)
    }

    fn chunk(&mut self, data: &[u8]) -> Result<(), FirmwareError> {
        self.open(None)?;
        self.port.write(data)?;
        self.state.add_downloaded(data.len() as u32);
        Ok(())
    }
}

impl OtaWorker {
    fn on_begin(&mut self, correlation_id: u64, begin: &FirmwareUpdateBeginCommand) {
        if !self.state.phase().accepts_new_update() {
            self.signal_failed(correlation_id, ErrorCode::Conflict, "update already in progress");
            return;
        }
        let url = begin.url.as_str().to_string();
        self.state.start(&url);
        self.signal_started(correlation_id);
        self.hold.engage();
        let outcome = self.run_on_worker_thread(&url);
        match outcome {
            Ok(written) => self.on_written(correlation_id, written),
            Err(error) => {
                self.hold.release();
                self.port.abort();
                self.state.fail(error);
                log::warn!("firmware update failed: {} ({url})", error.as_str());
                self.signal_failed(correlation_id, ErrorCode::ExecutionFailed, error.as_str());
            }
        }
    }

    fn run_on_worker_thread(&self, url: &str) -> Result<u32, FirmwareError> {
        let (port, source, state, url) = (
            Arc::clone(&self.port),
            Arc::clone(&self.source),
            Arc::clone(&self.state),
            url.to_string(),
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = match dali2rust_bsp::esp_thread::try_spawn_named_stack(
            c"ota-run",
            dali2rust_bsp::std_thread_stack::OTA_UPDATE_STACK,
            move || {
                let _ = tx.send(run(port.as_ref(), source.as_ref(), state.as_ref(), &url));
            },
        ) {
            Ok(handle) => handle,
            Err(e) => {
                log::error!("OTA: update thread spawn failed: {e:?}");
                return Err(FirmwareError::NoMemory);
            }
        };
        let outcome = rx.recv();
        let _ = handle.join();
        outcome.unwrap_or(Err(FirmwareError::Write))
    }

    fn on_written(&self, correlation_id: u64, written: u32) {
        self.state.set_phase(OtaPhase::ReadyToReboot);
        log::info!("firmware update written ({written} B), rebooting");
        self.signal_succeeded(correlation_id);
        // sleep-ok: documented product timing (`ADR-024`).
        std::thread::sleep(REBOOT_GRACE);
        self.port.reboot();
        self.hold.release();
    }

    fn signal_started(&self, correlation_id: u64) {
        self.publish(correlation_id, OperationWorkerSignalEvent::started(correlation_id));
    }

    fn signal_succeeded(&self, correlation_id: u64) {
        self.publish(correlation_id, OperationWorkerSignalEvent::succeeded(correlation_id));
    }

    fn signal_failed(&self, correlation_id: u64, code: ErrorCode, message: &str) {
        self.publish(
            correlation_id,
            OperationWorkerSignalEvent::failed(correlation_id, code, message),
        );
    }

    fn publish(&self, correlation_id: u64, signal: OperationWorkerSignalEvent) {
        let envelope = event_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.bus_id.0,
            Some(Origin::Api),
            signal,
        );
        let _ = publish_required(
            &self.publisher,
            BusChannel::Events,
            BusFrame::event(envelope),
            &REQUIRED_PUBLISH_BACKOFF_MS,
            REQUIRED_PUBLISH_UNCAPPED,
            "ota-worker-signal",
        );
    }
}

fn run(
    port: &dyn FirmwareUpdatePort,
    source: &dyn FirmwareImageSource,
    state: &OtaState,
    url: &str,
) -> Result<u32, FirmwareError> {
    let mut sink = SlotSink {
        port,
        state,
        opened: false,
    };
    let written = source.fetch(url, &mut sink)?;
    state.set_phase(OtaPhase::Finishing);
    port.finish()?;
    Ok(written)
}

pub struct OtaWorkerSeams {
    pub port: Arc<dyn FirmwareUpdatePort>,
    pub source: Arc<dyn FirmwareImageSource>,
    pub state: Arc<OtaState>,
    pub hold: Arc<MaintenanceHold>,
}

pub fn spawn_ota_worker(
    rx: BusSubscriberRx,
    publisher: BusPublisher,
    bus_id: BusId,
    seams: OtaWorkerSeams,
) -> std::thread::JoinHandle<()> {
    dali2rust_bsp::esp_thread::spawn_named_stack_in(
        c"ota-worker",
        dali2rust_bsp::std_thread_stack::OTA_LISTENER_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        move || {
            let mut worker = OtaWorker {
                publisher,
                bus_id,
                port: seams.port,
                source: seams.source,
                state: seams.state,
                hold: seams.hold,
            };
            while let Ok(frame) = rx.recv() {
                if let BusFrame::Command(command) = frame {
                    let correlation_id = command.meta.correlation_id;
                    dispatch_ota_command(&command.payload, &mut worker, correlation_id);
                }
            }
        },
    )
}
