use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::bus::{command_envelope, event_envelope};
use dali2rust_contracts::msg::{DaliEventPayload, OperationRegistryResetCommand, Origin};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;

pub const BUS_ROUTER_LOG_TARGET: &str = "dali2rust_bus::bus";

const PARK_CORRELATION: u64 = 0x7061_726B;
const PARK_DEADLINE: Duration = Duration::from_secs(10);
const PARK_LIMIT: Duration = Duration::from_secs(30);
const FILL_BOUND: usize = 4096;
const FILLER_WIRE_ADDRESS: u8 = 0xFE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RouterPark {
    Running,
    Armed,
    Parked,
    Released,
}

struct RouterParking {
    state: Mutex<RouterPark>,
    changed: Condvar,
}

static ROUTER_PARKING: RouterParking = RouterParking {
    state: Mutex::new(RouterPark::Running),
    changed: Condvar::new(),
};

static HOLD_SESSION: Mutex<()> = Mutex::new(());

static PARKING_LOGGER_INSTALLED: OnceLock<()> = OnceLock::new();

impl RouterParking {
    fn state(&self) -> MutexGuard<'_, RouterPark> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, next: RouterPark) {
        *self.state() = next;
        self.changed.notify_all();
    }

    fn hold_the_router_while_parked(&self) {
        let mut state = self.state();
        if *state != RouterPark::Armed {
            return;
        }
        *state = RouterPark::Parked;
        self.changed.notify_all();
        let _released = self
            .changed
            .wait_timeout_while(state, PARK_LIMIT, |s| *s == RouterPark::Parked)
            .unwrap_or_else(PoisonError::into_inner);
    }

    fn wait_until_parked(&self) -> bool {
        let (state, _) = self
            .changed
            .wait_timeout_while(self.state(), PARK_DEADLINE, |s| *s == RouterPark::Armed)
            .unwrap_or_else(PoisonError::into_inner);
        *state == RouterPark::Parked
    }
}

struct RouterParkingLogger;

impl log::Log for RouterParkingLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.target() == BUS_ROUTER_LOG_TARGET
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) && names_the_park_command(record) {
            ROUTER_PARKING.hold_the_router_while_parked();
        }
    }

    fn flush(&self) {}
}

static ROUTER_PARKING_LOGGER: RouterParkingLogger = RouterParkingLogger;

fn names_the_park_command(record: &log::Record) -> bool {
    record
        .args()
        .to_string()
        .ends_with(&format!("corr={PARK_CORRELATION}"))
}

fn install_the_parking_logger() {
    PARKING_LOGGER_INSTALLED.get_or_init(|| {
        log::set_logger(&ROUTER_PARKING_LOGGER).expect(
            "holding the events ingress full needs the process logger, and another logger \
             was installed first",
        );
        log::set_max_level(log::LevelFilter::Warn);
    });
}

fn park_command() -> BusFrame {
    BusFrame::command(command_envelope(
        SOURCE_ID_UNSPECIFIED,
        PARK_CORRELATION,
        BusId::default().0,
        Some(Origin::Internal),
        OperationRegistryResetCommand {},
    ))
}

fn unheard_filler_event() -> BusFrame {
    BusFrame::event(event_envelope(
        SOURCE_ID_UNSPECIFIED,
        0,
        BusId::default().0,
        Some(Origin::Internal),
        DaliEventPayload {
            wire_address: FILLER_WIRE_ADDRESS,
            command: 0,
            repeat_count: 1,
        },
    ))
}

fn park_the_router(publisher: &BusPublisher) {
    ROUTER_PARKING.set(RouterPark::Armed);
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, park_command()),
        PublishResult::Queued,
        "the commands ingress refused the command that parks the router"
    );
    assert!(
        ROUTER_PARKING.wait_until_parked(),
        "the bus router never logged the DeliveryRejected no confirmation subscriber took \
         for corr={PARK_CORRELATION} (inject_delivery_rejected in dali2rust-bus, target \
         {BUS_ROUTER_LOG_TARGET}, only on a bus's 1st, 2nd, 4th… such rejection). No \
         subscriber may handle OperationRegistryResetCommand, and the bus may have no \
         confirmation subscriber"
    );
}

fn fill_until_refused(publisher: &BusPublisher) {
    for _ in 0..FILL_BOUND {
        match publisher.try_publish(BusChannel::Events, unheard_filler_event()) {
            PublishResult::Queued => {}
            PublishResult::DroppedIngressFull => return,
            other => panic!("the filler event was refused for another reason: {other:?}"),
        }
    }
    panic!(
        "the events ingress took {FILL_BOUND} frames without refusing one, so the parked \
         router is still draining it"
    );
}

pub struct HeldEventsIngress {
    _session: MutexGuard<'static, ()>,
}

impl HeldEventsIngress {
    pub fn release(&self) {
        ROUTER_PARKING.set(RouterPark::Released);
    }
}

impl Drop for HeldEventsIngress {
    fn drop(&mut self) {
        ROUTER_PARKING.set(RouterPark::Released);
    }
}

pub fn hold_the_events_ingress_full(publisher: &BusPublisher) -> HeldEventsIngress {
    install_the_parking_logger();
    let held = HeldEventsIngress {
        _session: HOLD_SESSION.lock().unwrap_or_else(PoisonError::into_inner),
    };
    park_the_router(publisher);
    fill_until_refused(publisher);
    held
}
