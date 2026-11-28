use core::hash::{Hash, Hasher};
use core::mem::MaybeUninit;
use std::collections::hash_map::DefaultHasher;
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
    fixed_text_32, fixed_text_64, BusCommandPayload, BusEventPayload, ClusterForgetNodeCommand, ClusterStateChangedEvent,
    ClusterSubject, DeliveryStatus, ErrorCode, FixedText32, FixedText64,
};
use dali2rust_contracts::{CORRELATION_NONE, SOURCE_ID_UNSPECIFIED};
use dali2rust_domain::registry::{
    ClusterCopyReadPort, ClusterLampReadPort, ClusterPartReadPort, ClusterRecordReadPort, ClusterSettingsReadPort,
    ClusterSettingsView, ClusterWire, AdapterReadPort, DaliSettingsReadPort, HomeAssistantSettingsReadPort,
};
use dali2rust_platform::cluster_link::{ClusterLink, ClusterLinkBinder, MAX_CLUSTER_DATAGRAM_BYTES};
use dali2rust_platform::liveness::monotonic_ms;
use dali2rust_platform::net::NetworkLink;

use crate::frame::{cluster_tag, encode, DatagramBuffer, Frame, Header, Hello, Pulse, NODE_COUNT};
use crate::info::{ClusterInfo, ClusterInfoReadPort};
use crate::lamps::{ClusterLamps, ClusterLampsReadPort};
use crate::presence::{ListenWindow, OwnFacts, OwnMode, HALF_CLOCK_MS};

use super::announce::{reached, Announcer, OwnInfo, Turn};
use super::page_slot::PageSlot;
use super::counters::{bump, ClusterCounters};
use super::execute::{ClusterExecution, ExecutionReads};
use super::hold::ForeignHold;
use super::incoming::CommandIntake;
use super::lamp_announce::{LampAnnouncer, OwnLamps, Subject, ANNOUNCED_ADAPTER};
use super::outgoing::{placed_outgoing, Outgoing};
use super::receiver::{run_receiver, LinkHandoff};
use super::round::BlockRound;
use super::shared::{BlockState, ClusterPresence, ClusterPresenceReadPort, Forgotten, Settled};

mod block;
mod command;
mod forward;

pub use command::{
    CLUSTER_BUSY, CLUSTER_COMMAND_UNSUPPORTED, CLUSTER_NOT_RUNNING, CLUSTER_NO_ADDRESS, CLUSTER_OFF, CLUSTER_SEND_FAILED,
    CONTROLLER_PASSIVE,
};

pub const NODE_HEARD: &str = "node_heard";

pub const CLUSTER_WORKER_HANDLED_EVENTS: &[&str] = &[
    "ClusterSettingsChangedEvent",
    "IpAddressAssignedEvent",
    "DaliSettingsChangedEvent",
    "DaliTargetStateAppliedEvent",
    "DaliLevelTransitionAppliedEvent",
    "DaliSceneRecalledEvent",
    "DaliStopFadeAppliedEvent",
    "DaliObservedFrameEvent",
    "RuntimeStateChangedEvent",
    "VirtualLampChangedEvent",
    "GroupMatrixChangedEvent",
    "PhysicalDeviceChangedEvent",
    "HomeAssistantSettingsChangedEvent",
];


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
    pub records: Arc<dyn ClusterRecordReadPort>,
    pub parts: Arc<dyn ClusterPartReadPort>,
    pub copies: Arc<dyn ClusterCopyReadPort>,
    pub adapters: Arc<dyn AdapterReadPort>,
    pub info: Arc<ClusterInfo>,
    pub lamps: Arc<dyn ClusterLampReadPort>,
    pub lamp_cache: Arc<ClusterLamps>,
}

pub struct ClusterWorkerThreads {
    pub sender: JoinHandle<()>,
    pub receiver: JoinHandle<()>,
}

pub struct ClusterInboxes {
    pub commands_and_events: BusSubscriberRx,
    pub confirmations: BusSubscriberRx,
}

pub fn spawn_cluster_worker(inboxes: ClusterInboxes, ports: ClusterWorkerPorts) -> ClusterWorkerThreads {
    let (handoff_tx, handoff_rx) = mpsc::channel();
    let (presence, counters) = (Arc::clone(&ports.presence), Arc::clone(&ports.counters));
    let caches = (Arc::clone(&ports.info), Arc::clone(&ports.lamp_cache));
    let execution = execution(&ports);
    let settings = Arc::clone(&ports.settings);
    let hold = Arc::new(ForeignHold::new());
    let intake = CommandIntake {
        execution: Arc::clone(&execution),
        settings,
        parts: Arc::clone(&ports.parts),
        confirmations: inboxes.confirmations,
        hold: Arc::clone(&hold),
    };
    let receiver = spawn_named_stack_in(
        c"cluster_rx",
        std_thread_stack::EVENT_WORKER_STACK,
        StackHome::ExternalOnXip,
        move || run_receiver(handoff_rx, (presence, caches.0, caches.1), counters, intake),
    );
    let inbox = inboxes.commands_and_events;
    let sender = spawn_named_stack_in(
        c"cluster_tx",
        std_thread_stack::EVENT_WORKER_STACK,
        StackHome::ExternalOnXip,
        move || run_sender(inbox, &ports, Links { handoff: handoff_tx, execution, hold }),
    );
    ClusterWorkerThreads { sender, receiver }
}

fn execution(ports: &ClusterWorkerPorts) -> Arc<ClusterExecution> {
    let reads = ExecutionReads {
        parts: Arc::clone(&ports.parts),
        settings: Arc::clone(&ports.settings),
        adapters: Arc::clone(&ports.adapters),
    };
    let bus = (ports.publisher.clone(), ports.bus_id);
    Arc::new(ClusterExecution::new(bus, reads, Arc::clone(&ports.counters)))
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
    ClusterCommandCommand(command) => node.on_command(correlation_id, command),
}

struct Links {
    handoff: Sender<LinkHandoff>,
    execution: Arc<ClusterExecution>,
    hold: Arc<ForeignHold>,
}

fn run_sender(inbox: BusSubscriberRx, ports: &ClusterWorkerPorts, links: Links) {
    let mut node = NodeSender::new(ports, links);
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
            wire_protection: self.settings.wire_protection,
            forwards_foreign: self.settings.forwarding.foreign_master,
            running_since_ms: None,
        }
    }
}

struct NodeSender<'a> {
    ports: &'a ClusterWorkerPorts,
    handoff: Sender<LinkHandoff>,
    execution: Arc<ClusterExecution>,
    hold: Arc<ForeignHold>,
    outgoing: PsramBox<Outgoing>,
    mode: OwnMode,
    buffer: PsramBox<DatagramBuffer>,
    link: Option<Bound>,
    wants_link: bool,
    seq: u32,
    next_slot_ms: u32,
    digest: u64,
    announcer: Announcer,
    info_digest: u64,
    round: BlockRound,
    next_copy_ms: u32,
    lamps_digest: u64,
    lamp_announcer: LampAnnouncer,
    page_slot: PageSlot,
}

impl<'a> NodeSender<'a> {
    fn new(ports: &'a ClusterWorkerPorts, Links { handoff, execution, hold }: Links) -> Self {
        let now_ms = monotonic_ms();
        let digest = ports.presence.digest(now_ms);
        let buffer = datagram_buffer();
        Self {
            ports,
            handoff,
            execution,
            hold,
            outgoing: placed_outgoing(),
            mode: OwnMode::Off,
            buffer,
            link: None,
            wants_link: false,
            seq: 0,
            next_slot_ms: now_ms,
            digest,
            announcer: Announcer::new(),
            info_digest: 0,
            round: BlockRound::new(),
            next_copy_ms: now_ms,
            lamps_digest: 0,
            lamp_announcer: LampAnnouncer::new(),
            page_slot: PageSlot::default(),
        }
    }

    fn period_ms(&self) -> u32 {
        self.ports.presence.timing().pulse_period_ms()
    }

    fn advance(&mut self, now_ms: u32) {
        let reading = Reading::of(self.ports);
        self.reconcile(&reading, now_ms);
        let (listened_ms, renamed) = self.listened_ms(&reading, now_ms);
        let base = self.base_mode(&reading);
        let settled = self.ports.presence.settle(reading.identity(), base, listened_ms, now_ms);
        self.mode = settled.mode;
        if renamed {
            self.ports.info.clear();
            self.ports.lamp_cache.clear();
        }
        if settled.entered_running || self.slot_due(now_ms) {
            self.on_slot(&reading, settled, now_ms);
        }
        self.release_foreign(&reading, now_ms);
        self.carry_commands(&reading, now_ms);
        let running = settled.mode == OwnMode::Running;
        self.keep_block(reading.settings.node_number, running, now_ms);
        self.announce(&reading, running, now_ms);
        self.ports.presence.prune(now_ms);
        self.publish_if_changed(now_ms);
    }

    fn announce(&mut self, reading: &Reading, running: bool, now_ms: u32) {
        let hello = self.ports.presence.take_hello_heard() && running;
        let timing = self.ports.presence.timing();
        let split = self.ports.presence.own_status().block == BlockState::Contested;
        let turn = Turn { running, node: reading.settings.node_number, now_ms, hello, split, timing };
        let own = OwnInfo { records: &*self.ports.records, parts: &*self.ports.parts, copies: &*self.ports.copies };
        self.announcer.plan(turn, own);
        self.lamp_announcer.plan(turn, OwnLamps { lamps: &*self.ports.lamps, parts: &*self.ports.parts });
        if self.page_slot.held(now_ms) {
            return;
        }
        if self.announcer.page_due(now_ms) {
            self.send_info_page(reading, now_ms);
        } else if self.lamp_announcer.page_due(now_ms) {
            self.send_lamp_page(reading, now_ms);
        }
    }

    fn send_info_page(&mut self, reading: &Reading, now_ms: u32) {
        self.page_slot.take(now_ms);
        let header = self.next_header(reading);
        let written = self.announcer.write_page(&mut self.buffer, &header, now_ms);
        let closes_set = written.is_ok() && !self.announcer.paging();
        if self.transmit(written.ok()) && closes_set {
            bump(&self.ports.counters.info_sets_sent_total);
        }
    }

    fn send_lamp_page(&mut self, reading: &Reading, now_ms: u32) {
        self.page_slot.take(now_ms);
        let header = self.next_header(reading);
        let written = self.lamp_announcer.write_page(&mut self.buffer, &header, now_ms);
        self.transmit(written.ok());
    }

    fn listened_ms(&mut self, reading: &Reading, now_ms: u32) -> (u32, bool) {
        let Some(bound) = self.link.as_mut() else {
            return (0, false);
        };
        let claim = reading.claim();
        let renamed = bound.claim.1 != claim.1;
        if bound.claim != claim {
            bound.claim = claim;
            bound.listen = ListenWindow::opened_at(now_ms);
        }
        (bound.listen.listened_ms(now_ms), renamed)
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
            let left = |at_ms: u32| if reached(now_ms, at_ms) { 0 } else { at_ms.wrapping_sub(now_ms) };
            let paced = |at_ms: u32| self.page_slot.clamp(now_ms, at_ms);
            let info = self.announcer.deadline().map(paced).map_or(u32::MAX, left);
            let lamps = self.lamp_announcer.deadline().map(paced).map_or(u32::MAX, left);
            let commands = self.outgoing.deadline().map_or(u32::MAX, left);
            let held = self.hold.line().deadline().map_or(u32::MAX, left);
            let block = self.block_deadline().map_or(u32::MAX, left);
            let soonest = left(self.next_slot_ms).min(info).min(lamps).min(commands).min(held).min(block);
            Duration::from_millis(u64::from(soonest))
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
            self.ports.info.clear();
            self.ports.lamp_cache.clear();
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
        if self.link.is_none() {
            return;
        }
        let header = self.next_header(reading);
        let written = encode(&header, frame, &mut self.buffer).ok();
        self.transmit(written);
    }

    fn next_header(&mut self, reading: &Reading) -> Header {
        let header = Header {
            cluster_tag: cluster_tag(&reading.settings.cluster_name),
            sender_mac: reading.mac.unwrap_or_default(),
            boot_epoch: self.ports.presence.draw_boot_epoch(),
            node_no: reading.settings.node_number,
            seq: self.seq,
        };
        self.seq = self.seq.wrapping_add(1);
        header
    }

    fn transmit(&self, written: Option<usize>) -> bool {
        let link = self.link.as_ref().map(|bound| &bound.link);
        let bytes = written.and_then(|len| self.buffer.get(..len));
        let sent = link.zip(bytes).is_some_and(|(link, bytes)| link.send(bytes).is_ok());
        let counter = if sent { &self.ports.counters.datagrams_sent_total } else { &self.ports.counters.send_failures_total };
        bump(counter);
        sent
    }

    fn publish_if_changed(&mut self, now_ms: u32) {
        let digest = self.ports.presence.digest(now_ms);
        if digest != self.digest && self.publish(ClusterSubject::Status) {
            self.digest = digest;
        }
        let info_digest = self.info_digest(now_ms);
        if info_digest != self.info_digest && self.publish(ClusterSubject::Info) {
            self.info_digest = info_digest;
        }
        let lamps_digest = self.lamps_digest(now_ms);
        if lamps_digest != self.lamps_digest && self.publish(ClusterSubject::Lamps) {
            self.lamps_digest = lamps_digest;
        }
    }

    fn lamps_digest(&self, now_ms: u32) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.ports.lamp_cache.changes(now_ms).hash(&mut hasher);
        for node in 0..NODE_COUNT {
            let fresh = self.ports.lamp_cache.lamps_held(node).map(|held| held.fresh);
            (fresh, fresh.is_some() && self.ports.presence.member_heard(node, now_ms)).hash(&mut hasher);
        }
        hasher.finish()
    }

    fn info_digest(&self, now_ms: u32) -> u64 {
        let mut hasher = DefaultHasher::new();
        (self.ports.info.changes(now_ms), self.announcer.announced()).hash(&mut hasher);
        for node in 0..NODE_COUNT {
            let held = self.ports.info.info_held(node).is_some();
            (held && self.ports.presence.member_heard(node, now_ms)).hash(&mut hasher);
        }
        hasher.finish()
    }

    fn publish(&self, subject: ClusterSubject) -> bool {
        let changed = ClusterStateChangedEvent { subject };
        let envelope = event_envelope(SOURCE_ID_UNSPECIFIED, CORRELATION_NONE, self.ports.bus_id.0, None, changed);
        let frame = BusFrame::event(envelope);
        publish_or_drop(&self.ports.publisher, BusChannel::Events, frame, "cluster-state") == PublishResult::Queued
    }

    fn take(&mut self, frame: &BusFrame) {
        match frame {
            BusFrame::Command(command) => dispatch_cluster_command(&command.payload, self, command.meta.correlation_id),
            BusFrame::Event(event) => {
                self.note_lamps_moved(&event.payload);
                match &event.payload {
                    BusEventPayload::ClusterSettingsChangedEvent(_) => self.on_settings_changed(),
                    fact => self.forward(fact),
                }
            }
            _ => {}
        }
    }

    fn note_lamps_moved(&mut self, payload: &BusEventPayload) {
        let now_ms = monotonic_ms();
        for &subject in lamp_subjects(payload) {
            self.lamp_announcer.note_moved(subject, now_ms);
        }
    }

    fn on_settings_changed(&mut self) {
        self.announcer.note_settings_changed();
        self.next_slot_ms = monotonic_ms();
    }

    fn on_forget(&mut self, correlation_id: u64, forget: &ClusterForgetNodeCommand) {
        let outcome = self.ports.presence.forget(forget.node, monotonic_ms());
        if outcome == Forgotten::Removed {
            self.ports.info.forget(forget.node);
            self.ports.lamp_cache.forget(forget.node);
        }
        let product_error = (outcome == Forgotten::StillHeard).then_some((ErrorCode::Conflict, NODE_HEARD));
        confirm(&self.ports.publisher, correlation_id, product_error);
    }
}

fn lamp_subjects(payload: &BusEventPayload) -> &'static [Subject] {
    const STATE: &[Subject] = &[Subject::State];
    const BOTH: &[Subject] = &[Subject::State, Subject::Info];
    const INFO: &[Subject] = &[Subject::Info];
    match payload {
        BusEventPayload::RuntimeStateChangedEvent(changed) if changed.adapter_id == ANNOUNCED_ADAPTER => STATE,
        BusEventPayload::GroupMatrixChangedEvent(changed) if changed.adapter_id == ANNOUNCED_ADAPTER => STATE,
        BusEventPayload::ClusterSettingsChangedEvent(_) => STATE,
        BusEventPayload::DaliTargetStateAppliedEvent(_)
        | BusEventPayload::DaliLevelTransitionAppliedEvent(_)
        | BusEventPayload::DaliSceneRecalledEvent(_)
        | BusEventPayload::DaliStopFadeAppliedEvent(_)
        | BusEventPayload::DaliObservedFrameEvent(_) => STATE,
        BusEventPayload::VirtualLampChangedEvent(changed) if changed.adapter_id == ANNOUNCED_ADAPTER => BOTH,
        BusEventPayload::PhysicalDeviceChangedEvent(changed) if changed.adapter_id == ANNOUNCED_ADAPTER => BOTH,
        BusEventPayload::HomeAssistantSettingsChangedEvent(_) => INFO,
        _ => &[],
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
