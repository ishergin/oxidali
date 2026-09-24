use std::time::Duration;

use dali2rust_adapters::dali::transport::esp_idf::EspIdfDaliTransport;
use dali2rust_platform::dali::{DaliTransport, TransferOutcome};
use esp_idf_svc::hal::gpio::{InputPin, OutputPin};

use dali2rust_bsp::esp32p4::pins;

const OPCODE_QUERY_STATUS: u8 = 0x90;

const PROBE_SHORTS: [u8; 4] = [0, 1, 2, 3];

const SWEEP_PERIOD: Duration = Duration::from_secs(10);

fn query_status_frame(short: u8) -> u16 {
    u16::from(((short << 1) | 1) as u8) << 8 | u16::from(OPCODE_QUERY_STATUS)
}

fn describe(outcome: TransferOutcome) -> String {
    match outcome {
        TransferOutcome::Answer(b) => format!("ANSWER 0x{b:02x}"),
        TransferOutcome::NoAnswer => "no answer".into(),
        TransferOutcome::Collision => "COLLISION".into(),
        TransferOutcome::BusBusy => "bus busy".into(),
        TransferOutcome::ForeignInWindow => "foreign traffic in window".into(),
        TransferOutcome::CorruptedInWindow => "corrupted in window".into(),
    }
}

fn sweep(transport: &mut EspIdfDaliTransport) {
    let mut answered = 0usize;
    for short in PROBE_SHORTS {
        let frame = query_status_frame(short);
        match transport.exchange_frame(frame, true) {
            Ok(outcome) => {
                if matches!(outcome, TransferOutcome::Answer(_)) {
                    answered += 1;
                }
                log::info!(
                    "dali tx: QUERY STATUS sa{short} frame=0x{frame:04x} -> {}",
                    describe(outcome)
                );
            }
            Err(e) => log::warn!("dali tx: sa{short} frame=0x{frame:04x} -> error {e:?}"),
        }
    }
    log::info!(
        "dali tx: sweep done, {answered}/{} answered — confirm the frames on the WB sniffer, \
         not here",
        PROBE_SHORTS.len()
    );
}

pub fn start(tx: impl OutputPin + 'static, rx: impl InputPin + 'static) -> bool {
    let mut transport = match EspIdfDaliTransport::try_new(tx, rx) {
        Ok(t) => t,
        Err(e) => {
            log::error!(
                "dali: PHY init FAILED on tx=GPIO{} rx=GPIO{}: {e:?}",
                pins::DALI_TX_GPIO,
                pins::DALI_RX_GPIO
            );
            return false;
        }
    };
    log::info!(
        "dali: PHY up on tx=GPIO{} rx=GPIO{}",
        pins::DALI_TX_GPIO,
        pins::DALI_RX_GPIO
    );

    std::thread::Builder::new()
        .name("dali-tx-probe".into())
        .stack_size(dali2rust_bsp::std_thread_stack::COMMAND_WORKER_STACK)
        .spawn(move || loop {
            // sleep-ok: deliberate probe cadence, see SWEEP_PERIOD.
            std::thread::sleep(SWEEP_PERIOD);
            sweep(&mut transport);
        })
        .is_ok()
}
