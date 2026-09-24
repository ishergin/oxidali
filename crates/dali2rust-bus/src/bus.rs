use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::backends::std_mpsc::MpscSender;
use crate::channels::{Receiver, RecvTimeoutError, Sender};
use crate::counters::{BusCounters, ChannelCounters, SubscriberCounters};
use crate::frame::BusFrame;
use crate::publish::PublishResult;

const BUS_POLL_INTERVAL_MS: u32 = 10;
const COMMAND_DRAIN_BATCH: usize = 32;

const BUS_TASK_STACK: usize = 8 * 1024;

pub(crate) type BusSender = MpscSender<BusFrame>;

pub type BusSubscriberRx = std::sync::mpsc::Receiver<BusFrame>;

pub type BusTaskHandle = JoinHandle<()>;

struct RouteTable {
    targets: Vec<u16>,
}

const MAX_ROUTED_SUBSCRIBERS: usize = 16;

impl RouteTable {
    fn build(
        names: &'static [&'static str],
        handled_sets: &[&'static [&'static str]],
        registrar_fn: &str,
    ) -> Self {
        assert!(
            handled_sets.len() <= MAX_ROUTED_SUBSCRIBERS,
            "too many routed subscribers for the u16 route mask"
        );
        let mut targets = vec![0u16; names.len()];
        for (sub, handled) in handled_sets.iter().enumerate() {
            for name in *handled {
                let vi = names.iter().position(|n| n == name).unwrap_or_else(|| {
                    panic!("{registrar_fn}: {name} is not a payload variant of this channel (subscriber {sub})")
                });
                targets[vi] |= 1 << sub;
            }
        }
        Self { targets }
    }

    fn mask_for(&self, variant_index: usize) -> u16 {
        self.targets.get(variant_index).copied().unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusChannel {
    Commands,
    Confirmations,
    Events,
}

pub const DEFAULT_CONFIRMATION_SLOTS: usize = 24;

#[derive(Clone, Copy, Debug)]
pub struct BusConfig {
    pub commands_ingress: usize,
    pub confirmations_ingress: usize,
    pub events_ingress: usize,
    pub confirmation_slots: usize,
    pub confirmation_timeout_ms: u64,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            commands_ingress: 128,
            confirmations_ingress: 32,
            events_ingress: 64,
            confirmation_slots: DEFAULT_CONFIRMATION_SLOTS,
            confirmation_timeout_ms: 2000,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BusTaskStats {
    pub loops_idle: u64,
    pub commands_drained: u64,
    pub confirmations_drained: u64,
    pub events_drained: u64,
}

#[derive(Default)]
struct AtomicChannelCounters {
    publish_attempted: AtomicU32,
    publish_queued: AtomicU32,
    ingress_overflow: AtomicU32,
    oversize_rejected: AtomicU32,
    kind_mismatch: AtomicU32,
}

impl AtomicChannelCounters {
    fn snapshot(&self) -> ChannelCounters {
        ChannelCounters {
            publish_attempted: self.publish_attempted.load(Ordering::Relaxed),
            publish_queued: self.publish_queued.load(Ordering::Relaxed),
            ingress_overflow: self.ingress_overflow.load(Ordering::Relaxed),
            oversize_rejected: self.oversize_rejected.load(Ordering::Relaxed),
            kind_mismatch: self.kind_mismatch.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct AtomicSubscriberCounters {
    delivered: AtomicU32,
    receiver_overflow: AtomicU32,
    name: std::sync::OnceLock<&'static str>,
}

impl AtomicSubscriberCounters {
    fn snapshot(&self) -> SubscriberCounters {
        SubscriberCounters {
            delivered: self.delivered.load(Ordering::Relaxed),
            receiver_overflow: self.receiver_overflow.load(Ordering::Relaxed),
            name: self.name.get().copied().unwrap_or(""),
        }
    }
}

struct AtomicBusCounters {
    commands: AtomicChannelCounters,
    confirmations: AtomicChannelCounters,
    events: AtomicChannelCounters,
    commands_unrouted: AtomicU32,
    delivery_rejected_dropped: AtomicU32,
    command_subscribers: Vec<AtomicSubscriberCounters>,
    confirmation_subscribers: Vec<AtomicSubscriberCounters>,
    event_subscribers: Vec<AtomicSubscriberCounters>,
}

impl AtomicBusCounters {
    fn with_subscriber_counts(commands: usize, confirmations: usize, events: usize) -> Self {
        Self {
            commands: AtomicChannelCounters::default(),
            confirmations: AtomicChannelCounters::default(),
            events: AtomicChannelCounters::default(),
            commands_unrouted: AtomicU32::default(),
            delivery_rejected_dropped: AtomicU32::default(),
            command_subscribers: (0..commands)
                .map(|_| AtomicSubscriberCounters::default())
                .collect(),
            confirmation_subscribers: (0..confirmations)
                .map(|_| AtomicSubscriberCounters::default())
                .collect(),
            event_subscribers: (0..events)
                .map(|_| AtomicSubscriberCounters::default())
                .collect(),
        }
    }

    fn snapshot(&self) -> BusCounters {
        BusCounters {
            commands: self.commands.snapshot(),
            confirmations: self.confirmations.snapshot(),
            events: self.events.snapshot(),
            commands_unrouted: self.commands_unrouted.load(Ordering::Relaxed),
            delivery_rejected_dropped: self.delivery_rejected_dropped.load(Ordering::Relaxed),
            command_subscribers: self
                .command_subscribers
                .iter()
                .map(AtomicSubscriberCounters::snapshot)
                .collect(),
            confirmation_subscribers: self
                .confirmation_subscribers
                .iter()
                .map(AtomicSubscriberCounters::snapshot)
                .collect(),
            event_subscribers: self
                .event_subscribers
                .iter()
                .map(AtomicSubscriberCounters::snapshot)
                .collect(),
        }
    }
}

#[derive(Default)]
struct AtomicBusTaskStats {
    loops_idle: AtomicU32,
    commands_drained: AtomicU32,
    confirmations_drained: AtomicU32,
    events_drained: AtomicU32,
}

impl AtomicBusTaskStats {
    fn snapshot(&self) -> BusTaskStats {
        BusTaskStats {
            loops_idle: self.loops_idle.load(Ordering::Relaxed) as u64,
            commands_drained: self.commands_drained.load(Ordering::Relaxed) as u64,
            confirmations_drained: self.confirmations_drained.load(Ordering::Relaxed) as u64,
            events_drained: self.events_drained.load(Ordering::Relaxed) as u64,
        }
    }
}

pub trait CommandArrivalObserver: Send + Sync {
    fn observe_command(
        &self,
        meta: &dali2rust_contracts::msg::BusEnvelope,
        payload: &dali2rust_contracts::msg::BusCommandPayload,
    );
}

struct BusInner {
    cmd_ingress: BusSender,
    conf_ingress: BusSender,
    event_ingress: BusSender,
    counters: Arc<AtomicBusCounters>,
    command_arrival: Option<Arc<dyn CommandArrivalObserver>>,
}

#[derive(Clone)]
pub struct BusPublisher {
    inner: Arc<BusInner>,
}

impl BusPublisher {
    pub fn counters_snapshot(&self) -> BusCounters {
        self.inner.counters.snapshot()
    }

    pub fn event_inbox_probe(&self, index: usize) -> EventInboxProbe {
        EventInboxProbe {
            counters: Arc::clone(&self.inner.counters),
            index,
        }
    }

    pub fn try_publish(&self, channel: BusChannel, frame: BusFrame) -> PublishResult {
        let kind_ok = matches!(
            (&channel, &frame),
            (BusChannel::Commands, BusFrame::Command(_))
                | (BusChannel::Confirmations, BusFrame::Confirmation(_))
                | (BusChannel::Events, BusFrame::Event(_))
        );
        let ch = match channel {
            BusChannel::Commands => &self.inner.counters.commands,
            BusChannel::Confirmations => &self.inner.counters.confirmations,
            BusChannel::Events => &self.inner.counters.events,
        };
        ch.publish_attempted.fetch_add(1, Ordering::Relaxed);
        if !kind_ok {
            ch.kind_mismatch.fetch_add(1, Ordering::Relaxed);
            return PublishResult::RejectedKindMismatch;
        }
        if frame_exceeds_wire_limit(&frame) {
            ch.oversize_rejected.fetch_add(1, Ordering::Relaxed);
            return PublishResult::RejectedFrameTooLarge;
        }
        self.observe_arrival(&frame);

        let tx = match channel {
            BusChannel::Commands => &self.inner.cmd_ingress,
            BusChannel::Confirmations => &self.inner.conf_ingress,
            BusChannel::Events => &self.inner.event_ingress,
        };
        queue_frame(tx, ch, frame)
    }

    fn observe_arrival(&self, frame: &BusFrame) {
        let (Some(observer), BusFrame::Command(ce)) = (&self.inner.command_arrival, frame) else {
            return;
        };
        observer.observe_command(&ce.meta, &ce.payload);
    }
}

fn queue_frame(tx: &BusSender, ch: &AtomicChannelCounters, frame: BusFrame) -> PublishResult {
    match tx.try_send(frame) {
        Ok(()) => {
            ch.publish_queued.fetch_add(1, Ordering::Relaxed);
            PublishResult::Queued
        }
        Err(_) => {
            ch.ingress_overflow.fetch_add(1, Ordering::Relaxed);
            PublishResult::DroppedIngressFull
        }
    }
}

fn frame_exceeds_wire_limit(frame: &BusFrame) -> bool {
    match frame.postcard_wire_len() {
        Some(wire_len) => wire_len > crate::frame::MAX_BUS_FRAME_BYTES,
        None => true,
    }
}

pub struct BusHost {
    publisher: BusPublisher,
    task: Option<BusTaskHandle>,
    task_stats: Arc<AtomicBusTaskStats>,
    shutdown: Arc<AtomicBool>,
}

impl Drop for BusHost {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

impl BusHost {
    pub fn spawn<T>(
        config: BusConfig,
        register: impl FnOnce(&mut BusRegistrar<BusSender, BusSubscriberRx>) -> T,
    ) -> (Self, BusPublisher, T) {
        Self::spawn_with_command_observer(config, None, register)
    }

    pub fn spawn_with_command_observer<T>(
        config: BusConfig,
        command_arrival: Option<Arc<dyn CommandArrivalObserver>>,
        register: impl FnOnce(&mut BusRegistrar<BusSender, BusSubscriberRx>) -> T,
    ) -> (Self, BusPublisher, T) {
        let task_stats = Arc::new(AtomicBusTaskStats::default());
        let (cmd_tx, cmd_rx) = crate::backends::std_mpsc::create_channel(config.commands_ingress);
        let (conf_tx, conf_rx) =
            crate::backends::std_mpsc::create_channel(config.confirmations_ingress);
        let (ev_tx, ev_rx) = crate::backends::std_mpsc::create_channel(config.events_ingress);

        let (reg, registered) = make_registrar_and_register(register);
        let cmd_route = RouteTable::build(
            dali2rust_contracts::msg::COMMAND_VARIANT_NAMES,
            &reg.cmd_handled,
            "subscribe_commands",
        );
        let ev_route = RouteTable::build(
            dali2rust_contracts::msg::EVENT_VARIANT_NAMES,
            &reg.ev_handled,
            "subscribe_events",
        );
        let counters = make_counters(reg.cmd_subs.len(), reg.conf_subs.len(), &reg.ev_names);
        let publisher = make_publisher(&cmd_tx, &conf_tx, &ev_tx, &counters, command_arrival);
        let shutdown = Arc::new(AtomicBool::new(false));
        let task = launch_bus_task(
            cmd_rx,
            conf_rx,
            ev_rx,
            reg.cmd_subs,
            reg.conf_subs,
            reg.ev_subs,
            (cmd_route, ev_route),
            &counters,
            &task_stats,
            &shutdown,
        );
        let host = BusHost {
            publisher: publisher.clone(),
            task: Some(task),
            task_stats,
            shutdown,
        };
        (host, publisher, registered)
    }

    pub fn join(mut self) -> std::thread::Result<()> {
        self.shutdown.store(true, Ordering::Relaxed);
        match self.task.take() {
            Some(task) => task.join(),
            None => Ok(()),
        }
    }

    pub fn task_stats(&self) -> BusTaskStats {
        self.task_stats.snapshot()
    }

    pub fn counters_snapshot(&self) -> BusCounters {
        self.publisher.inner.counters.snapshot()
    }
}

type CmdRx = std::sync::mpsc::Receiver<BusFrame>;

fn make_registrar_and_register<T>(
    register: impl FnOnce(&mut BusRegistrar<BusSender, CmdRx>) -> T,
) -> (BusRegistrar<BusSender, CmdRx>, T) {
    let make_channel = |cap: usize| {
        let (tx, rx) = std::sync::mpsc::sync_channel(cap);
        (MpscSender::new(tx), rx)
    };
    let mut reg = BusRegistrar::new(make_channel);
    let registered = register(&mut reg);
    (reg, registered)
}

fn make_counters(
    cmds: usize,
    confs: usize,
    ev_names: &[&'static str],
) -> Arc<AtomicBusCounters> {
    let counters = AtomicBusCounters::with_subscriber_counts(cmds, confs, ev_names.len());
    for (slot, name) in counters.event_subscribers.iter().zip(ev_names) {
        let _ = slot.name.set(name);
    }
    Arc::new(counters)
}

fn make_publisher(
    cmd_tx: &BusSender,
    conf_tx: &BusSender,
    ev_tx: &BusSender,
    counters: &Arc<AtomicBusCounters>,
    command_arrival: Option<Arc<dyn CommandArrivalObserver>>,
) -> BusPublisher {
    let inner = Arc::new(BusInner {
        cmd_ingress: cmd_tx.clone(),
        conf_ingress: conf_tx.clone(),
        event_ingress: ev_tx.clone(),
        counters: Arc::clone(counters),
        command_arrival,
    });
    BusPublisher { inner }
}

#[allow(
    clippy::too_many_arguments,
    reason = "forwards all channel rx/subscriber lists straight into bus_task_loop"
)]
fn launch_bus_task<R: Receiver<BusFrame> + Send + 'static>(
    cmd_rx: R,
    conf_rx: R,
    ev_rx: R,
    cmd_subs: Vec<BusSender>,
    conf_subs: Vec<BusSender>,
    ev_subs: Vec<BusSender>,
    routes: (RouteTable, RouteTable),
    counters: &Arc<AtomicBusCounters>,
    task_stats: &Arc<AtomicBusTaskStats>,
    shutdown: &Arc<AtomicBool>,
) -> BusTaskHandle {
    let counters_clone = Arc::clone(counters);
    let stats_clone = Arc::clone(task_stats);
    let shutdown_clone = Arc::clone(shutdown);
    dali2rust_bsp::esp_thread::spawn_named_stack(c"bus_task", BUS_TASK_STACK, move || {
        bus_task_loop(
            cmd_rx,
            conf_rx,
            ev_rx,
            cmd_subs,
            conf_subs,
            ev_subs,
            routes,
            counters_clone,
            stats_clone,
            &shutdown_clone,
        );
    })
}

pub struct BusRegistrar<S, R> {
    cmd_subs: Vec<S>,
    cmd_handled: Vec<&'static [&'static str]>,
    conf_subs: Vec<S>,
    ev_subs: Vec<S>,
    ev_handled: Vec<&'static [&'static str]>,
    ev_names: Vec<&'static str>,
    make_channel: fn(usize) -> (S, R),
}

impl<S: Sender<BusFrame>, R> BusRegistrar<S, R> {
    pub(crate) fn new(make_channel: fn(usize) -> (S, R)) -> Self {
        Self {
            cmd_subs: Vec::new(),
            cmd_handled: Vec::new(),
            conf_subs: Vec::new(),
            ev_subs: Vec::new(),
            ev_handled: Vec::new(),
            ev_names: Vec::new(),
            make_channel,
        }
    }

    pub fn subscribe_commands(
        &mut self,
        inbox_capacity: usize,
        handled: &'static [&'static str],
    ) -> R {
        let (tx, rx) = (self.make_channel)(inbox_capacity);
        self.cmd_subs.push(tx);
        self.cmd_handled.push(handled);
        rx
    }

    pub fn subscribe_commands_and_events(
        &mut self,
        inbox_capacity: usize,
        cmd_handled: &'static [&'static str],
        ev_handled: &'static [&'static str],
    ) -> R
    where
        S: Clone,
    {
        let (tx, rx) = (self.make_channel)(inbox_capacity);
        self.cmd_subs.push(tx.clone());
        self.cmd_handled.push(cmd_handled);
        self.ev_subs.push(tx);
        self.ev_handled.push(ev_handled);
        self.ev_names.push(UNNAMED_SUBSCRIBER);
        rx
    }

    pub fn subscribe_confirmations(&mut self, inbox_capacity: usize) -> R {
        let (tx, rx) = (self.make_channel)(inbox_capacity);
        self.conf_subs.push(tx);
        rx
    }

    pub fn subscribe_events(
        &mut self,
        inbox_capacity: usize,
        handled: &'static [&'static str],
    ) -> R {
        self.subscribe_events_indexed(inbox_capacity, handled).0
    }

    pub fn subscribe_events_indexed(
        &mut self,
        inbox_capacity: usize,
        handled: &'static [&'static str],
    ) -> (R, usize) {
        self.subscribe_events_named_indexed(inbox_capacity, handled, UNNAMED_SUBSCRIBER)
    }

    pub fn subscribe_events_named(
        &mut self,
        inbox_capacity: usize,
        handled: &'static [&'static str],
        name: &'static str,
    ) -> R {
        self.subscribe_events_named_indexed(inbox_capacity, handled, name).0
    }

    pub fn subscribe_events_named_indexed(
        &mut self,
        inbox_capacity: usize,
        handled: &'static [&'static str],
        name: &'static str,
    ) -> (R, usize) {
        let (tx, rx) = (self.make_channel)(inbox_capacity);
        let index = self.ev_subs.len();
        self.ev_subs.push(tx);
        self.ev_handled.push(handled);
        self.ev_names.push(name);
        (rx, index)
    }

    pub fn subscribe_commands_and_events_named(
        &mut self,
        inbox_capacity: usize,
        cmd_handled: &'static [&'static str],
        ev_handled: &'static [&'static str],
        name: &'static str,
    ) -> R
    where
        S: Clone,
    {
        let (tx, rx) = (self.make_channel)(inbox_capacity);
        self.cmd_subs.push(tx.clone());
        self.cmd_handled.push(cmd_handled);
        self.ev_subs.push(tx);
        self.ev_handled.push(ev_handled);
        self.ev_names.push(name);
        rx
    }
}

pub const UNNAMED_SUBSCRIBER: &str = "unnamed";

#[derive(Clone)]
pub struct EventInboxProbe {
    counters: Arc<AtomicBusCounters>,
    index: usize,
}

impl EventInboxProbe {
    pub fn dropped_total(&self) -> u32 {
        self.counters
            .event_subscribers
            .get(self.index)
            .map_or(0, |c| c.receiver_overflow.load(Ordering::Relaxed))
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "bus task loop receives all channel rx/tx/subscribers"
)]
fn bus_task_loop<R: Receiver<BusFrame>, S: Sender<BusFrame>>(
    cmd_rx: R,
    conf_rx: R,
    ev_rx: R,
    cmd_subs: Vec<S>,
    conf_subs: Vec<S>,
    ev_subs: Vec<S>,
    routes: (RouteTable, RouteTable),
    counters: Arc<AtomicBusCounters>,
    stats: Arc<AtomicBusTaskStats>,
    shutdown: &AtomicBool,
) {
    let (cmd_route, ev_route) = routes;
    let mut backlog = false;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        let wait = if backlog { 0 } else { BUS_POLL_INTERVAL_MS };
        match cmd_rx.recv_timeout(wait) {
            Ok(frame) => process_commands(
                frame,
                &cmd_rx,
                &cmd_subs,
                &cmd_route,
                &counters,
                &stats,
                &conf_subs,
            ),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if !backlog {
                    stats.loops_idle.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        backlog = drain_and_update_stats(
            &conf_rx, &ev_rx, &conf_subs, &ev_subs, &ev_route, &counters, &stats,
        );
    }
}

fn drain_and_update_stats<R: Receiver<BusFrame>, S: Sender<BusFrame>>(
    conf_rx: &R,
    ev_rx: &R,
    conf_subs: &[S],
    ev_subs: &[S],
    ev_route: &RouteTable,
    counters: &AtomicBusCounters,
    stats: &AtomicBusTaskStats,
) -> bool {
    let n_conf = drain_non_blocking(conf_rx, conf_subs, &counters.confirmation_subscribers);
    let n_ev = drain_events_routed(ev_rx, ev_subs, ev_route, counters);
    if n_conf > 0 || n_ev > 0 {
        stats
            .confirmations_drained
            .fetch_add(n_conf as u32, Ordering::Relaxed);
        stats
            .events_drained
            .fetch_add(n_ev as u32, Ordering::Relaxed);
    }
    n_conf >= COMMAND_DRAIN_BATCH as u64 || n_ev >= COMMAND_DRAIN_BATCH as u64
}

fn process_commands<R: Receiver<BusFrame>, S: Sender<BusFrame>>(
    first_frame: BusFrame,
    cmd_rx: &R,
    cmd_subs: &[S],
    route: &RouteTable,
    counters: &Arc<AtomicBusCounters>,
    stats: &Arc<AtomicBusTaskStats>,
    conf_subs: &[S],
) {
    dispatch_command(&first_frame, cmd_subs, route, counters, conf_subs);
    let mut drained: u64 = 1;
    while drained < COMMAND_DRAIN_BATCH as u64 {
        let Ok(frame) = cmd_rx.try_recv() else {
            break;
        };
        drained += 1;
        dispatch_command(&frame, cmd_subs, route, counters, conf_subs);
    }
    stats
        .commands_drained
        .fetch_add(drained as u32, Ordering::Relaxed);
}

fn fan_out_frame<S: Sender<BusFrame>>(
    frame: &BusFrame,
    subs: &[S],
    counters: &[AtomicSubscriberCounters],
) -> usize {
    debug_assert_eq!(
        subs.len(),
        counters.len(),
        "subscriber count must match counter count"
    );
    let mut delivered = 0;
    for (i, tx) in subs.iter().enumerate() {
        match tx.try_send(frame.clone()) {
            Ok(()) => {
                counters[i].delivered.fetch_add(1, Ordering::Relaxed);
                delivered += 1;
            }
            Err(_) => {
                counters[i]
                    .receiver_overflow
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    delivered
}

fn dispatch_command<S: Sender<BusFrame>>(
    frame: &BusFrame,
    subs: &[S],
    route: &RouteTable,
    counters: &AtomicBusCounters,
    conf_subs: &[S],
) {
    let BusFrame::Command(ce) = frame else {
        return;
    };
    let mask = route.mask_for(ce.payload.variant_index());
    if mask == 0 {
        counters.commands_unrouted.fetch_add(1, Ordering::Relaxed);
        inject_delivery_rejected(conf_subs, ce, counters);
        return;
    }
    if deliver_to_targets(frame, mask, subs, &counters.command_subscribers) == 0 {
        inject_delivery_rejected(conf_subs, ce, counters);
    }
}

fn deliver_to_targets<S: Sender<BusFrame>>(
    frame: &BusFrame,
    mask: u16,
    subs: &[S],
    counters: &[AtomicSubscriberCounters],
) -> usize {
    let mut delivered = 0;
    let mut remaining = mask;
    while remaining != 0 {
        let i = remaining.trailing_zeros() as usize;
        remaining &= remaining - 1;
        if i >= subs.len() {
            break;
        }
        match subs[i].try_send(frame.clone()) {
            Ok(()) => {
                counters[i].delivered.fetch_add(1, Ordering::Relaxed);
                delivered += 1;
            }
            Err(_) => {
                counters[i]
                    .receiver_overflow
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    delivered
}

fn inject_delivery_rejected<S: Sender<BusFrame>>(
    conf_subs: &[S],
    cmd: &dali2rust_contracts::msg::CommandEnvelope,
    counters: &AtomicBusCounters,
) {
    let Some(cf) = dali2rust_contracts::bus::synthetic_delivery_rejected_envelope(cmd) else {
        return;
    };
    let delivered = fan_out_frame(
        &BusFrame::confirmation(cf),
        conf_subs,
        &counters.confirmation_subscribers,
    );
    if delivered == 0 {
        let n = counters
            .delivery_rejected_dropped
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        if n.is_power_of_two() {
            log::warn!(
                "bus: synthetic DeliveryRejected accepted by no confirmation subscriber, total={n} corr={}",
                cmd.meta.correlation_id
            );
        }
    }
}

fn drain_non_blocking<R: Receiver<BusFrame>, S: Sender<BusFrame>>(
    rx: &R,
    subs: &[S],
    subscriber_counters: &[AtomicSubscriberCounters],
) -> u64 {
    let mut n: u64 = 0;
    while n < COMMAND_DRAIN_BATCH as u64 {
        let Ok(frame) = rx.try_recv() else {
            break;
        };
        n += 1;
        let _ = fan_out_frame(&frame, subs, subscriber_counters);
    }
    n
}

fn drain_events_routed<R: Receiver<BusFrame>, S: Sender<BusFrame>>(
    ev_rx: &R,
    ev_subs: &[S],
    route: &RouteTable,
    counters: &AtomicBusCounters,
) -> u64 {
    let mut n: u64 = 0;
    while n < COMMAND_DRAIN_BATCH as u64 {
        let Ok(frame) = ev_rx.try_recv() else {
            break;
        };
        n += 1;
        route_event(&frame, ev_subs, route, counters);
    }
    n
}

fn route_event<S: Sender<BusFrame>>(
    frame: &BusFrame,
    subs: &[S],
    route: &RouteTable,
    counters: &AtomicBusCounters,
) {
    let BusFrame::Event(ev) = frame else {
        return;
    };
    let mask = route.mask_for(ev.payload.variant_index());
    if mask == 0 {
        return;
    }
    let _ = deliver_to_targets(frame, mask, subs, &counters.event_subscribers);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample_command_frame() -> BusFrame {
        BusFrame::command(dali2rust_contracts::bus::command_envelope(0, 1, 0, None, dali2rust_contracts::msg::DaliCommandPayload { wire_address: 1, command: 2, repeat_count: 1, raw_mode: false, raw_expects_backward: false }))
    }

    #[test]
    fn one_command_turn_has_a_finite_drain_budget() {
        use crate::backends::std_mpsc::create_channel;

        let queued_after_budget = 3;
        let queued_commands = COMMAND_DRAIN_BATCH - 1 + queued_after_budget;
        let (tx, rx) = create_channel(queued_commands);
        let first = sample_command_frame();
        for _ in 0..queued_commands {
            tx.try_send(sample_command_frame()).expect("queue command");
        }
        let counters = Arc::new(AtomicBusCounters::with_subscriber_counts(0, 0, 0));
        let stats = Arc::new(AtomicBusTaskStats::default());
        let route = RouteTable::build(
            dali2rust_contracts::msg::COMMAND_VARIANT_NAMES,
            &[],
            "test",
        );
        let subscribers: Vec<BusSender> = Vec::new();

        process_commands(
            first,
            &rx,
            &subscribers,
            &route,
            &counters,
            &stats,
            &subscribers,
        );

        assert_eq!(stats.snapshot().commands_drained, COMMAND_DRAIN_BATCH as u64);
        let remaining = std::iter::from_fn(|| rx.try_recv().ok()).count();
        assert_eq!(remaining, queued_after_budget);
    }

    #[test]
    fn every_ingress_drain_stops_at_its_budget_and_reports_the_backlog() {
        use crate::backends::std_mpsc::create_channel;

        let queued_after_budget = 5;
        let queued = COMMAND_DRAIN_BATCH + queued_after_budget;
        let (conf_tx, conf_rx) = create_channel(queued);
        let (ev_tx, ev_rx) = create_channel(queued);
        for _ in 0..queued {
            conf_tx.try_send(sample_command_frame()).expect("queue confirmation");
            ev_tx.try_send(sample_command_frame()).expect("queue event");
        }

        let counters = AtomicBusCounters::with_subscriber_counts(0, 0, 0);
        let stats = AtomicBusTaskStats::default();
        let ev_route =
            RouteTable::build(dali2rust_contracts::msg::EVENT_VARIANT_NAMES, &[], "test");
        let subscribers: Vec<BusSender> = Vec::new();

        let backlog = drain_and_update_stats(
            &conf_rx,
            &ev_rx,
            &subscribers,
            &subscribers,
            &ev_route,
            &counters,
            &stats,
        );

        assert!(backlog, "a saturated drain must tell the loop not to park");
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.confirmations_drained, COMMAND_DRAIN_BATCH as u64);
        assert_eq!(snapshot.events_drained, COMMAND_DRAIN_BATCH as u64);
        assert_eq!(
            std::iter::from_fn(|| conf_rx.try_recv().ok()).count(),
            queued_after_budget
        );
        assert_eq!(
            std::iter::from_fn(|| ev_rx.try_recv().ok()).count(),
            queued_after_budget
        );
    }

    #[test]
    fn a_quiet_turn_reports_no_backlog() {
        use crate::backends::std_mpsc::create_channel;

        let (_conf_tx, conf_rx) = create_channel(4);
        let (ev_tx, ev_rx) = create_channel(4);
        ev_tx.try_send(sample_command_frame()).expect("queue event");

        let counters = AtomicBusCounters::with_subscriber_counts(0, 0, 0);
        let stats = AtomicBusTaskStats::default();
        let ev_route =
            RouteTable::build(dali2rust_contracts::msg::EVENT_VARIANT_NAMES, &[], "test");
        let subscribers: Vec<BusSender> = Vec::new();

        let backlog = drain_and_update_stats(
            &conf_rx,
            &ev_rx,
            &subscribers,
            &subscribers,
            &ev_route,
            &counters,
            &stats,
        );

        assert!(!backlog);
        assert_eq!(stats.snapshot().events_drained, 1);
    }

    #[test]
    fn ingress_overflow_when_full() {
        let config = BusConfig {
            commands_ingress: 1,
            ..BusConfig::default()
        };
        let (_host, publisher, ()) = BusHost::spawn(
            config,
            |reg| {
                let _rx = reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
            },
        );

        let f = sample_command_frame();
        let mut saw_drop = false;
        for _ in 0..256 {
            match publisher.try_publish(BusChannel::Commands, f.clone()) {
                PublishResult::DroppedIngressFull => {
                    saw_drop = true;
                    break;
                }
                PublishResult::Queued => {}
                other => panic!("unexpected publish result: {other:?}"),
            }
        }
        assert!(
            saw_drop,
            "expected ingress backpressure with capacity-1 commands ingress"
        );
    }

    #[test]
    fn receiver_overflow_increments_when_inbox_full() {
        let config = BusConfig {
            commands_ingress: 8,
            ..BusConfig::default()
        };
        let (host, publisher, ()) = BusHost::spawn(
            config,
            |reg| {
                let _rx = reg.subscribe_commands(1, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
            },
        );

        let f = sample_command_frame();
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, f.clone()),
            PublishResult::Queued
        );
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, f.clone()),
            PublishResult::Queued
        );
        dali2rust_test_support::wait_until(
            || host.counters_snapshot().command_subscribers[0].receiver_overflow >= 1,
            Duration::from_millis(500),
        );
        let c = host.counters_snapshot();
        assert_eq!(c.command_subscribers.len(), 1);
        assert!(
            c.command_subscribers[0].receiver_overflow >= 1,
            "expected receiver overflow when inbox=1 and two frames"
        );
    }

    #[test]
    fn kind_mismatch_rejected() {
        let (_host, publisher, ()) = BusHost::spawn(
            BusConfig::default(),
            |reg| {
                let _rx = reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
            },
        );
        let ev = BusFrame::event(dali2rust_contracts::bus::event_envelope(0, 0, 0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::IpAddressAssignedEvent::from_ip_text("10.0.0.1")));
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, ev),
            PublishResult::RejectedKindMismatch
        );
    }

    #[test]
    fn owner_inbox_full_emits_synthetic_delivery_rejected() {
        use dali2rust_contracts::msg::DeliveryStatus;

        let (_host, publisher, (_owner_rx, other_rx, conf_rx)) = BusHost::spawn(
            BusConfig::default(),
            |reg| {
                (
                    reg.subscribe_commands(1, &["DaliCommandPayload"]),
                    reg.subscribe_commands(16, &["OperationBeginCommand"]),
                    reg.subscribe_confirmations(8),
                )
            },
        );

        let frame = BusFrame::command(dali2rust_contracts::bus::command_envelope(0, 42, 1, None, dali2rust_contracts::msg::DaliCommandPayload { wire_address: 2, command: 5, repeat_count: 1, raw_mode: false, raw_expects_backward: false }));

        assert_eq!(
            publisher.try_publish(BusChannel::Commands, frame.clone()),
            PublishResult::Queued
        );
        assert_eq!(
            publisher.try_publish(BusChannel::Commands, frame.clone()),
            PublishResult::Queued
        );

        let conf_frame = conf_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("synthetic confirmation");
        let BusFrame::Confirmation(ce) = conf_frame else {
            panic!("expected confirmation frame");
        };
        assert_eq!(ce.status, DeliveryStatus::DeliveryRejected);
        assert_eq!(ce.meta.correlation_id, 42);

        assert!(
            other_rx.try_recv().is_err(),
            "non-owner must not receive a routed command"
        );
    }

    #[test]
    fn funneled_subscriber_receives_commands_and_events() {
        let (_host, publisher, rx) = BusHost::spawn(BusConfig::default(), |reg| {
            reg.subscribe_commands_and_events(8, &["DaliCommandPayload"], &["IpAddressAssignedEvent"])
        });

        assert_eq!(
            publisher.try_publish(BusChannel::Commands, sample_command_frame()),
            PublishResult::Queued
        );
        let ev = BusFrame::event(dali2rust_contracts::bus::event_envelope(0, 7, 0, Some(dali2rust_contracts::msg::Origin::Internal), dali2rust_contracts::msg::IpAddressAssignedEvent::from_ip_text("10.0.0.2")));
        assert_eq!(
            publisher.try_publish(BusChannel::Events, ev),
            PublishResult::Queued
        );

        let (mut saw_command, mut saw_event) = (false, false);
        for _ in 0..2 {
            match rx.recv_timeout(Duration::from_millis(500)).expect("frame") {
                BusFrame::Command(_) => saw_command = true,
                BusFrame::Event(_) => saw_event = true,
                BusFrame::Confirmation(_) => panic!("confirmation on funneled inbox"),
            }
        }
        assert!(saw_command, "funneled inbox must receive routed commands");
        assert!(saw_event, "funneled inbox must receive its declared events");
    }

    #[test]
    fn dropped_synthetic_delivery_rejected_is_counted() {
        use crate::backends::std_mpsc::create_channel;

        let counters = AtomicBusCounters::with_subscriber_counts(0, 1, 0);
        let BusFrame::Command(ce) = sample_command_frame() else {
            panic!("expected command frame");
        };

        let (full_tx, _full_rx) = create_channel::<BusFrame>(1);
        full_tx
            .try_send(sample_command_frame())
            .expect("fill capacity-1 inbox");
        inject_delivery_rejected(&[full_tx], &ce, &counters);
        let snap = counters.snapshot();
        assert_eq!(snap.delivery_rejected_dropped, 1);
        assert_eq!(snap.confirmation_subscribers[0].receiver_overflow, 1);

        let (free_tx, free_rx) = create_channel::<BusFrame>(1);
        inject_delivery_rejected(&[free_tx], &ce, &counters);
        assert_eq!(counters.snapshot().delivery_rejected_dropped, 1);
        assert!(matches!(free_rx.try_recv(), Ok(BusFrame::Confirmation(_))));
    }

    #[test]
    fn oversized_command_rejected_before_ingress() {
        let mut env = dali2rust_contracts::bus::command_envelope(u16::MAX, u64::MAX, u16::MAX, Some(dali2rust_contracts::msg::Origin::Api), dali2rust_contracts::msg::DaliCommandPayload { wire_address: 1, command: 2, repeat_count: 1, raw_mode: false, raw_expects_backward: false });
        env.meta.error = Some(Box::new(dali2rust_contracts::msg::ErrorPayload::with_details(
            dali2rust_contracts::msg::ErrorCode::UnsupportedCapability,
            "e".repeat(64),
            &[0xAB; 32],
        )));
        let f = BusFrame::command(env);
        assert!(f.postcard_wire_len().unwrap() > crate::frame::MAX_BUS_FRAME_BYTES);

        let (_host, publisher, ()) = BusHost::spawn(
            BusConfig::default(),
            |reg| {
                let _ = reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
            },
        );

        assert_eq!(
            publisher.try_publish(BusChannel::Commands, f),
            PublishResult::RejectedFrameTooLarge
        );
    }
}
