use core::mem::MaybeUninit;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use dali2rust_bsp::esp_thread::{spawn_named_stack_in, StackHome};
use dali2rust_bsp::psram::PsramBox;
use dali2rust_bsp::stack_probe::StackLowWater;
use dali2rust_bsp::std_thread_stack;
use dali2rust_bus::{
    publish_or_drop, recv_then_drain, wait_then_drain, BusChannel, BusFrame, BusId, BusPublisher,
    BusSubscriberRx, PublishResult, WorkerTurn,
};
use dali2rust_contracts::bus::{
    build_confirmation_envelope, build_confirmation_envelope_with_product_error, event_envelope,
};
use dali2rust_contracts::msg::{
    fixed_text_32, fixed_text_64, BusCommandPayload, ClusterForgetNodeCommand, ClusterStateChangedEvent,
    ClusterSubject, DeliveryStatus, ErrorCode, FixedText32, FixedText64,
};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_domain::registry::{
    ClusterSettingsReadPort, ClusterSettingsView, ClusterWire, DaliSettingsReadPort,
    HomeAssistantSettingsReadPort,
};
use dali2rust_platform::cluster_link::{ClusterLink, ClusterLinkBinder, MAX_CLUSTER_DATAGRAM_BYTES};
use dali2rust_platform::liveness::monotonic_ms;
use dali2rust_platform::net::NetworkLink;

use crate::frame::{cluster_tag, encode, DatagramBuffer, Frame, Header, Hello, Pulse};
use crate::presence::{ListenWindow, OwnFacts, OwnMode, HALF_CLOCK_MS};

use super::counters::{bump, ClusterCounters};
use super::receiver::{run_receiver, LinkHandoff};
use super::shared::{ClusterPresence, Forgotten, Settled};

pub const NODE_HEARD: &str = "node_heard";

pub const CLUSTER_WORKER_HANDLED_EVENTS: &[&str] =
    &["ClusterSettingsChangedEvent", "IpAddressAssignedEvent", "DaliSettingsChangedEvent"];


static SENDER_STACK: StackLowWater = StackLowWater::new("cluster_tx", std_thread_stack::EVENT_WORKER_STACK);

pub struct ClusterWorkerPorts {
    pub publisher: BusPublisher,
    pub bus_id: BusId,
    pub settings: Arc<dyn ClusterSettingsReadPort>,
    pub home_assistant: Arc<dyn HomeAssistantSettingsReadPort>,
    pub role: Arc<dyn DaliSettingsReadPort>,
    pub network: Option<Arc<dyn NetworkLink>>,
    pub binder: Arc<dyn ClusterLinkBinder>,
    pub presence: Arc<ClusterPresence>,
    pub counters: Arc<ClusterCounters>,
}

pub struct ClusterWorkerThreads {
    pub sender: JoinHandle<()>,
    pub receiver: JoinHandle<()>,
}

pub fn spawn_cluster_worker(inbox: BusSubscriberRx, ports: ClusterWorkerPorts) -> ClusterWorkerThreads {
    let (handoff_tx, handoff_rx) = mpsc::channel();
    let (presence, counters) = (Arc::clone(&ports.presence), Arc::clone(&ports.counters));
    let receiver = spawn_named_stack_in(
        c"cluster_rx",
        std_thread_stack::EVENT_WORKER_STACK,
        StackHome::ExternalOnXip,
        move || run_receiver(handoff_rx, presence, counters),
    );
    let sender = spawn_named_stack_in(
        c"cluster_tx",
        std_thread_stack::EVENT_WORKER_STACK,
        StackHome::ExternalOnXip,
        move || run_sender(inbox, &ports, handoff_tx),
    );
    ClusterWorkerThreads { sender, receiver }
}

dali2rust_contracts::dispatch_bus_commands! {
    pub const CLUSTER_WORKER_HANDLED_COMMANDS;
    fn dispatch_cluster_command(
        payload: &BusCommandPayload,
        node: &mut NodeSender<'_>,
        correlation_id: u64,
    );
    payload = payload;
    ignored = {};
    ClusterForgetNodeCommand(forget) => node.on_forget(correlation_id, forget),
}

fn run_sender(inbox: BusSubscriberRx, ports: &ClusterWorkerPorts, handoff: Sender<LinkHandoff>) {
    let mut node = NodeSender::new(ports, handoff);
    SENDER_STACK.note("start");
    loop {
        node.advance(monotonic_ms());
        SENDER_STACK.note("advance");
        let turn = match node.wait(monotonic_ms()) {
            Some(timeout) => recv_then_drain(&inbox, timeout, |frame| node.take(&frame)),
            None => wait_then_drain(&inbox, |frame| node.take(&frame))
                .map_or(WorkerTurn::Disconnected, |()| WorkerTurn::Handled),
        };
        if turn == WorkerTurn::Disconnected {
            return;
        }
    }
}

struct Bound {
    link: Arc<dyn ClusterLink>,
    port: u16,
    listen: ListenWindow,
    claim: Claim,
}

type Claim = (u8, FixedText64);

struct Reading {
    settings: ClusterSettingsView,
    installation: FixedText32,
    mac: Option<[u8; 6]>,
    leased: bool,
    active: bool,
}

impl Reading {
    fn of(ports: &ClusterWorkerPorts) -> Self {
        let status = ports.network.as_ref().map(|link| link.status());
        Self {
            settings: ports.settings.cluster_settings_view(),
            installation: fixed_text_32(&ports.home_assistant.home_assistant_settings_view().controller_id),
            mac: ports.network.as_ref().and_then(|link| link.hardware_address()),
            leased: status.is_some_and(|status| status.ipv4.is_some()),
            active: ports.role.dali_settings_view().application_active,
        }
    }

    fn wanted_port(&self) -> Option<u16> {
        let reachable = self.settings.enabled && self.leased && self.mac.is_some();
        reachable.then_some(self.settings.port)
    }

    fn claim(&self) -> Claim {
        (self.settings.node_number, fixed_text_64(&self.settings.cluster_name))
    }

    const fn wire(&self) -> u8 {
        match self.settings.wire {
            ClusterWire::Own => self.settings.node_number,
            ClusterWire::SameAs(node) => node,
        }
    }

    fn identity(&self) -> OwnFacts {
        OwnFacts {
            cluster_name: fixed_text_64(&self.settings.cluster_name),
            cluster_tag: cluster_tag(&self.settings.cluster_name),
            installation: self.installation.clone(),
            mac: self.mac.unwrap_or_default(),
            node_no: self.settings.node_number,
            wire: self.wire(),
            running_since_ms: None,
        }
    }
}

struct NodeSender<'a> {
    ports: &'a ClusterWorkerPorts,
    handoff: Sender<LinkHandoff>,
    buffer: PsramBox<DatagramBuffer>,
    link: Option<Bound>,
    wants_link: bool,
    seq: u32,
    next_slot_ms: u32,
    digest: u64,
}

impl<'a> NodeSender<'a> {
    fn new(ports: &'a ClusterWorkerPorts, handoff: Sender<LinkHandoff>) -> Self {
        let now_ms = monotonic_ms();
        let digest = ports.presence.digest(now_ms);
        let buffer = datagram_buffer();
        Self { ports, handoff, buffer, link: None, wants_link: false, seq: 0, next_slot_ms: now_ms, digest }
    }

    fn period_ms(&self) -> u32 {
        self.ports.presence.timing().pulse_period_ms()
    }

    fn advance(&mut self, now_ms: u32) {
        let reading = Reading::of(self.ports);
        self.reconcile(&reading, now_ms);
        let listened_ms = self.listened_ms(&reading, now_ms);
        let base = self.base_mode(&reading);
        let settled = self.ports.presence.settle(reading.identity(), base, listened_ms, now_ms);
        if settled.entered_running || self.slot_due(now_ms) {
            self.on_slot(&reading, settled, now_ms);
        }
        self.ports.presence.prune(now_ms);
        self.publish_if_changed(now_ms);
    }

    fn listened_ms(&mut self, reading: &Reading, now_ms: u32) -> u32 {
        let Some(bound) = self.link.as_mut() else {
            return 0;
        };
        let claim = reading.claim();
        if bound.claim != claim {
            bound.claim = claim;
            bound.listen = ListenWindow::opened_at(now_ms);
        }
        bound.listen.listened_ms(now_ms)
    }

    fn base_mode(&self, reading: &Reading) -> Option<OwnMode> {
        if !reading.settings.enabled {
            return Some(OwnMode::Off);
        }
        if !reading.leased || reading.mac.is_none() {
            return Some(OwnMode::NoAddress);
        }
        if self.link.is_none() {
            return Some(OwnMode::BindFailed);
        }
        (!reading.active).then_some(OwnMode::Standby)
    }

    fn slot_due(&self, now_ms: u32) -> bool {
        now_ms.wrapping_sub(self.next_slot_ms) < HALF_CLOCK_MS
    }

    fn wait(&self, now_ms: u32) -> Option<Duration> {
        self.wants_link.then(|| {
            let left = self.next_slot_ms.wrapping_sub(now_ms);
            Duration::from_millis(u64::from(if left < HALF_CLOCK_MS { left } else { 0 }))
        })
    }

    fn reconcile(&mut self, reading: &Reading, now_ms: u32) {
        let wanted = reading.wanted_port();
        self.wants_link = wanted.is_some();
        match (self.link.as_ref().map(|bound| bound.port), wanted) {
            (Some(port), Some(want)) if port == want => {}
            (None, None) => {}
            (Some(_), None) => self.unbind(),
            (_, Some(want)) => self.bind(want, reading.claim(), now_ms),
        }
    }

    fn bind(&mut self, port: u16, claim: Claim, now_ms: u32) {
        self.unbind();
        self.next_slot_ms = now_ms;
        match self.ports.binder.bind(port) {
            Ok(link) => {
                let generation = self.ports.presence.link();
                let _ = self.handoff.send(LinkHandoff::Bound { link: Arc::clone(&link), generation });
                self.link = Some(Bound { link, port, listen: ListenWindow::opened_at(now_ms), claim });
            }
            Err(error) => {
                bump(&self.ports.counters.bind_failures_total);
                log::warn!("cluster: bind of port {port} failed: {error:?}");
                self.next_slot_ms = now_ms.wrapping_add(self.period_ms());
            }
        }
    }

    fn unbind(&mut self) {
        if self.link.take().is_some() {
            let _ = self.handoff.send(LinkHandoff::Unbound);
            self.ports.presence.clear();
        }
    }

    fn on_slot(&mut self, reading: &Reading, settled: Settled, now_ms: u32) {
        self.next_slot_ms = now_ms.wrapping_add(self.period_ms());
        if settled.mode.transmits() {
            self.send(reading, &Frame::Pulse(pulse(reading, settled.mode)));
        }
        if settled.entered_running {
            self.send(reading, &Frame::Hello(Hello { ha_face: false }));
        }
        if settled.mode == OwnMode::Standby {
            bump(&self.ports.counters.withheld_standby_total);
        }
    }

    fn send(&mut self, reading: &Reading, frame: &Frame<'_>) {
        let Some(bound) = self.link.as_ref() else {
            return;
        };
        let header = Header {
            cluster_tag: cluster_tag(&reading.settings.cluster_name),
            sender_mac: reading.mac.unwrap_or_default(),
            boot_epoch: self.ports.presence.draw_boot_epoch(),
            node_no: reading.settings.node_number,
            seq: self.seq,
        };
        self.seq = self.seq.wrapping_add(1);
        let written = encode(&header, frame, &mut self.buffer).ok();
        let sent = written.and_then(|len| self.buffer.get(..len)).map(|bytes| bound.link.send(bytes));
        let counter = match sent {
            Some(Ok(())) => &self.ports.counters.datagrams_sent_total,
            _ => &self.ports.counters.send_failures_total,
        };
        bump(counter);
    }

    fn publish_if_changed(&mut self, now_ms: u32) {
        let digest = self.ports.presence.digest(now_ms);
        if digest == self.digest {
            return;
        }
        let changed = ClusterStateChangedEvent { subject: ClusterSubject::Status };
        let envelope = event_envelope(SOURCE_ID_UNSPECIFIED, CORRELATION_NONE, self.ports.bus_id.0, None, changed);
        let frame = BusFrame::event(envelope);
        if publish_or_drop(&self.ports.publisher, BusChannel::Events, frame, "cluster-state") == PublishResult::Queued {
            self.digest = digest;
        }
    }

    fn take(&mut self, frame: &BusFrame) {
        if let BusFrame::Command(command) = frame {
            dispatch_cluster_command(&command.payload, self, command.meta.correlation_id);
        }
    }

    fn on_forget(&mut self, correlation_id: u64, forget: &ClusterForgetNodeCommand) {
        let outcome = self.ports.presence.forget(forget.node, monotonic_ms());
        let product_error = (outcome == Forgotten::StillHeard).then_some((ErrorCode::Conflict, NODE_HEARD));
        confirm(&self.ports.publisher, correlation_id, product_error);
    }
}

fn pulse(reading: &Reading, mode: OwnMode) -> Pulse<'_> {
    Pulse {
        cluster_name: &reading.settings.cluster_name,
        installation: &reading.installation,
        claim: mode.claim(),
        wire: reading.wire(),
        wire_protection: reading.settings.wire_protection,
    }
}

fn confirm(publisher: &BusPublisher, correlation_id: u64, product_error: Option<(ErrorCode, &str)>) {
    if correlation_id == CORRELATION_NONE {
        return;
    }
    let envelope = match product_error {
        None => build_confirmation_envelope(correlation_id, DeliveryStatus::Ok, 0, SOURCE_ID_UNSPECIFIED),
        Some(error) => build_confirmation_envelope_with_product_error(
            correlation_id,
            DeliveryStatus::ExecutionFailed,
            0,
            SOURCE_ID_UNSPECIFIED,
            Some(error),
        ),
    };
    publish_or_drop(publisher, BusChannel::Confirmations, BusFrame::confirmation(envelope), "cluster-confirm");
}

fn datagram_buffer() -> PsramBox<DatagramBuffer> {
    // SAFETY: `write_bytes` zeroes all `MAX_CLUSTER_DATAGRAM_BYTES` bytes, and a zero byte is a valid `u8`.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<DatagramBuffer>| {
            place.as_mut_ptr().cast::<u8>().write_bytes(0, MAX_CLUSTER_DATAGRAM_BYTES);
        })
    }
}
