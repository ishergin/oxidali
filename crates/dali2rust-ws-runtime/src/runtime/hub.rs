use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use dali2rust_api::ws::{
    error_frame, hello_frame, parse_client_request, subscription_ack_frame, Channel, ClientRequest,
    DropNotice, SubscriptionAck, V1_CHANNELS,
};
use dali2rust_contracts::msg::ErrorCode;
use dali2rust_bsp::log_ring::{LogRing, ARMED_LEVEL, LOG_REPLAY_LINES};
use dali2rust_platform::dali::SnifferTap;
use dali2rust_platform::logs::LogLevel;

use crate::counters::WsCounters;
use crate::runtime::sink::WsSink;

pub const MAX_WS_CLIENTS: usize = 4;

const CLIENT_QUEUE_DEPTH: usize = 32;

const MAX_CLIENT_QUEUE_BYTES: usize = 16 * 1024;

const DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Info;

const CHANNEL_COUNT: usize = V1_CHANNELS.len();

pub type ClientId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterRejected {
    CapacityExhausted,
    OriginRejected,
}

pub const CLOSE_POLICY_VIOLATION: u16 = 1008;

pub const CLOSE_TRY_AGAIN_LATER: u16 = 1013;

pub const fn close_code(reason: RegisterRejected) -> u16 {
    match reason {
        RegisterRejected::CapacityExhausted => CLOSE_TRY_AGAIN_LATER,
        RegisterRejected::OriginRejected => CLOSE_POLICY_VIOLATION,
    }
}

pub fn origin_allowed(origin: Option<&str>, host: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let Some(host) = host else {
        return false;
    };
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    !scheme.is_empty()
        && !authority.is_empty()
        && !authority.contains('/')
        && authority.eq_ignore_ascii_case(host)
}

#[derive(Debug, Clone, Copy)]
pub struct WsHubConfig {
    pub max_clients: usize,
    pub queue_depth: usize,
}

impl Default for WsHubConfig {
    fn default() -> Self {
        Self {
            max_clients: MAX_WS_CLIENTS,
            queue_depth: CLIENT_QUEUE_DEPTH,
        }
    }
}

fn most_verbose_log_level(clients: &[Client]) -> LogLevel {
    clients
        .iter()
        .filter_map(|client| client.log_level)
        .max()
        .unwrap_or(ARMED_LEVEL)
        .max(ARMED_LEVEL)
}

#[cfg(test)]
pub(crate) fn leaked_test_ring() -> &'static LogRing {
    // SAFETY: test-only leak; the ring must outlive the hub that borrows it.
    Box::leak(Box::new(LogRing::new()))
}

struct Client {
    id: ClientId,
    tx: std::sync::mpsc::SyncSender<Arc<str>>,
    closed: Arc<std::sync::atomic::AtomicBool>,
    queued_bytes: Arc<std::sync::atomic::AtomicUsize>,
    subscriptions: u16,
    dropped: [u32; CHANNEL_COUNT],
    dropped_global: u32,
    log_level: Option<LogLevel>,
}

impl Client {
    fn is_subscribed(&self, channel: Channel) -> bool {
        self.subscriptions & channel.bit() != 0
    }

    fn channels(&self) -> Vec<Channel> {
        V1_CHANNELS
            .iter()
            .copied()
            .filter(|c| self.is_subscribed(*c))
            .collect()
    }

    fn enqueue(&self, payload: Arc<str>) -> Result<(), std::sync::mpsc::TrySendError<Arc<str>>> {
        let len = payload.len();
        if self.queued_bytes.load(Ordering::Relaxed).saturating_add(len) > MAX_CLIENT_QUEUE_BYTES {
            return Err(std::sync::mpsc::TrySendError::Full(payload));
        }
        self.queued_bytes.fetch_add(len, Ordering::Relaxed);
        match self.tx.try_send(payload) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.queued_bytes.fetch_sub(len, Ordering::Relaxed);
                Err(e)
            }
        }
    }
}

pub struct WsHub {
    config: WsHubConfig,
    clients: Mutex<Vec<Client>>,
    arrived: std::sync::Condvar,
    next_id: AtomicU32,
    subscriptions: AtomicU16,
    counters: Arc<WsCounters>,
    tap: Arc<SnifferTap>,
    log_ring: &'static LogRing,
}

impl WsHub {
    pub fn new(
        config: WsHubConfig,
        counters: Arc<WsCounters>,
        tap: Arc<SnifferTap>,
        log_ring: &'static LogRing,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            clients: Mutex::new(Vec::new()),
            arrived: std::sync::Condvar::new(),
            next_id: AtomicU32::new(1),
            subscriptions: AtomicU16::new(0),
            counters,
            tap,
            log_ring,
        })
    }

    pub fn log_ring(&self) -> &'static LogRing {
        self.log_ring
    }

    pub fn counters(&self) -> &Arc<WsCounters> {
        &self.counters
    }

    pub fn register(&self, sink: Box<dyn WsSink>) -> Result<ClientId, RegisterRejected> {
        let mut clients = self.lock();
        if clients.len() >= self.config.max_clients {
            WsCounters::bump(&self.counters.upgrades_rejected_total, 1);
            return Err(RegisterRejected::CapacityExhausted);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = std::sync::mpsc::sync_channel(self.config.queue_depth);
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let queued_bytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        clients.push(Client {
            id,
            tx,
            closed: Arc::clone(&closed),
            queued_bytes: Arc::clone(&queued_bytes),
            subscriptions: 0,
            dropped: [0; CHANNEL_COUNT],
            dropped_global: 0,
            log_level: None,
        });
        self.counters.client_connected();
        self.arrived.notify_all();
        drop(clients);
        if spawn_sender_thread(id, sink, rx, closed, queued_bytes).is_err() {
            self.unregister(id);
            WsCounters::bump(&self.counters.upgrades_rejected_total, 1);
            return Err(RegisterRejected::CapacityExhausted);
        }
        self.send_direct(id, hello_frame());
        Ok(id)
    }

    pub fn unregister(&self, id: ClientId) {
        let mut clients = self.lock();
        let before = clients.len();
        clients.retain(|c| c.id != id);
        if before != clients.len() {
            self.counters.client_disconnected();
            self.refresh_gates(&clients);
        }
    }

    pub fn client_count(&self) -> usize {
        self.lock().len()
    }

    pub fn handle_client_text(&self, id: ClientId, text: &str) {
        let reply = match parse_client_request(text) {
            Ok(request) => self.apply_request(id, request),
            Err(err) => error_frame(err.code, &err.message),
        };
        self.send_direct(id, reply);
    }

    pub fn rejection_frame(reason: RegisterRejected) -> String {
        match reason {
            RegisterRejected::CapacityExhausted => error_frame(
                ErrorCode::WsClientsExhausted,
                "websocket client limit reached",
            ),
            RegisterRejected::OriginRejected => {
                error_frame(ErrorCode::WsOriginRejected, "origin not allowed")
            }
        }
    }

    fn apply_request(&self, id: ClientId, request: ClientRequest) -> String {
        let mut clients = self.lock();
        let Some(client) = clients.iter_mut().find(|c| c.id == id) else {
            return error_frame(ErrorCode::NotFound, "client is gone");
        };
        let mut replay = false;
        let (ack, channels) = match request {
            ClientRequest::Subscribe { channels, log_level } => {
                let wants_logs = channels.contains(&Channel::Logs);
                let first_logs_subscribe = wants_logs && client.log_level.is_none();
                for channel in channels {
                    client.subscriptions |= channel.bit();
                }
                if wants_logs {
                    client.log_level = Some(log_level.unwrap_or(DEFAULT_LOG_LEVEL));
                }
                replay = first_logs_subscribe;
                (SubscriptionAck::Subscribed, client.channels())
            }
            ClientRequest::Unsubscribe(list) => {
                for channel in list {
                    client.subscriptions &= !channel.bit();
                    client.dropped[channel.slot()] = 0;
                    if channel == Channel::Logs {
                        client.log_level = None;
                    }
                }
                (SubscriptionAck::Unsubscribed, client.channels())
            }
        };
        self.refresh_gates(&clients);
        drop(clients);
        if replay {
            self.log_ring.request_replay(LOG_REPLAY_LINES);
        }
        subscription_ack_frame(ack, &channels)
    }

    fn refresh_gates(&self, clients: &[Client]) {
        let mask = clients.iter().fold(0u16, |mask, c| mask | c.subscriptions);
        self.subscriptions.store(mask, Ordering::Relaxed);
        self.tap.set_enabled(mask & Channel::Sniffer.bit() != 0);
        self.log_ring.set_min_level(most_verbose_log_level(clients));
    }

    fn send_direct(&self, id: ClientId, text: String) {
        let clients = self.lock();
        if let Some(client) = clients.iter().find(|c| c.id == id) {
            let _ = client.enqueue(Arc::from(text));
        }
    }

    pub fn broadcast(&self, channel: Channel, frame: &str) {
        self.broadcast_counting(channel, frame, 1);
    }

    pub fn log_level_cohorts(&self) -> Vec<LogLevel> {
        let clients = self.lock();
        let mut levels: Vec<LogLevel> = clients
            .iter()
            .filter(|client| client.is_subscribed(Channel::Logs))
            .filter_map(|client| client.log_level)
            .collect();
        levels.sort_unstable();
        levels.dedup();
        levels
    }

    pub fn broadcast_to_log_level(&self, level: LogLevel, frame: &str, loss_weight: u32) {
        let mut clients = self.lock();
        let payload: Arc<str> = Arc::from(frame);
        let mut sent = 0u32;
        let mut dropped = 0u32;
        for client in clients
            .iter_mut()
            .filter(|c| c.is_subscribed(Channel::Logs) && c.log_level == Some(level))
        {
            match deliver(client, Channel::Logs, &payload, loss_weight) {
                Delivery::Sent => sent += 1,
                Delivery::Dropped => dropped += 1,
                Delivery::Gone => {}
            }
        }
        drop(clients);
        WsCounters::bump(&self.counters.events_sent_total, sent);
        WsCounters::bump(&self.counters.events_dropped_total, dropped);
    }

    pub fn broadcast_counting(&self, channel: Channel, frame: &str, loss_weight: u32) {
        if !self.any_subscriber(channel) {
            return;
        }
        let mut clients = self.lock();
        let payload: Arc<str> = Arc::from(frame);
        let mut sent = 0u32;
        let mut dropped = 0u32;
        for client in clients.iter_mut().filter(|c| c.is_subscribed(channel)) {
            match deliver(client, channel, &payload, loss_weight) {
                Delivery::Sent => sent += 1,
                Delivery::Dropped => dropped += 1,
                Delivery::Gone => {}
            }
        }
        drop(clients);
        WsCounters::bump(&self.counters.events_sent_total, sent);
        WsCounters::bump(&self.counters.events_dropped_total, dropped);
    }

    pub fn note_inbox_overflow(&self, count: u32) {
        if count == 0 {
            return;
        }
        WsCounters::bump(&self.counters.inbox_overflow_total, count);
        let mut clients = self.lock();
        for client in clients.iter_mut() {
            client.dropped_global = client.dropped_global.saturating_add(count);
            let _ = notify_inbox_loss(client);
        }
    }

    pub fn reap_closed(&self) {
        let mut clients = self.lock();
        let before = clients.len();
        clients.retain(|c| !c.closed.load(Ordering::Relaxed));
        let removed = before - clients.len();
        for _ in 0..removed {
            self.counters.client_disconnected();
        }
        if removed > 0 {
            self.refresh_gates(&clients);
        }
    }

    pub fn any_subscriber(&self, channel: Channel) -> bool {
        self.subscriptions.load(Ordering::Relaxed) & channel.bit() != 0
    }

    pub fn has_clients(&self) -> bool {
        WsCounters::load(&self.counters.clients) > 0
    }

    pub fn wait_for_clients(&self, timeout: std::time::Duration) {
        let guard = self.lock();
        if !guard.is_empty() {
            return;
        }
        let _ = self.arrived.wait_timeout(guard, timeout);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Client>> {
        self.clients
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Debug, Default)]
pub struct WsSessions {
    rows: Mutex<Vec<(i32, ClientId)>>,
}

impl WsSessions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, fd: i32, id: ClientId) {
        self.locked().push((fd, id));
    }

    pub fn lookup(&self, fd: i32) -> Option<ClientId> {
        self.locked()
            .iter()
            .find_map(|(row_fd, id)| (*row_fd == fd).then_some(*id))
    }

    pub fn close(&self, hub: &WsHub, fd: i32) {
        for id in self.forget(fd) {
            hub.unregister(id);
        }
    }

    fn forget(&self, fd: i32) -> Vec<ClientId> {
        let mut rows = self.locked();
        let mut ids = Vec::new();
        rows.retain(|(row_fd, id)| {
            if *row_fd == fd {
                ids.push(*id);
                false
            } else {
                true
            }
        });
        ids
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, Vec<(i32, ClientId)>> {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

enum Delivery {
    Sent,
    Dropped,
    Gone,
}

fn deliver(
    client: &mut Client,
    channel: Channel,
    payload: &Arc<str>,
    loss_weight: u32,
) -> Delivery {
    if !notify_inbox_loss(client) || !notify_channel_loss(client, channel) {
        return count_drop(client, channel, loss_weight);
    }
    match client.enqueue(Arc::clone(payload)) {
        Ok(()) => Delivery::Sent,
        Err(std::sync::mpsc::TrySendError::Full(_)) => count_drop(client, channel, loss_weight),
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
            client.closed.store(true, Ordering::Relaxed);
            Delivery::Gone
        }
    }
}

fn count_drop(client: &mut Client, channel: Channel, lost: u32) -> Delivery {
    client.dropped[channel.slot()] = client.dropped[channel.slot()].saturating_add(lost);
    Delivery::Dropped
}

fn push_notice(client: &mut Client, notice: &DropNotice) -> bool {
    match client.enqueue(Arc::from(notice.to_frame())) {
        Ok(()) => true,
        Err(std::sync::mpsc::TrySendError::Full(_)) => false,
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
            client.closed.store(true, Ordering::Relaxed);
            true
        }
    }
}

fn notify_inbox_loss(client: &mut Client) -> bool {
    let owed = client.dropped_global;
    if owed == 0 {
        return true;
    }
    let notice = DropNotice {
        channel: None,
        dropped_count: owed,
    };
    if !push_notice(client, &notice) {
        return false;
    }
    client.dropped_global = 0;
    true
}

fn notify_channel_loss(client: &mut Client, channel: Channel) -> bool {
    let owed = client.dropped[channel.slot()];
    if owed == 0 {
        return true;
    }
    let notice = DropNotice {
        channel: Some(channel),
        dropped_count: owed,
    };
    if !push_notice(client, &notice) {
        return false;
    }
    client.dropped[channel.slot()] = 0;
    true
}

fn spawn_sender_thread(
    id: ClientId,
    sink: Box<dyn WsSink>,
    rx: std::sync::mpsc::Receiver<Arc<str>>,
    closed: Arc<std::sync::atomic::AtomicBool>,
    queued_bytes: Arc<std::sync::atomic::AtomicUsize>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    dali2rust_bsp::esp_thread::try_spawn_named_stack_in(
        c"ws-client",
        dali2rust_bsp::std_thread_stack::EVENT_WORKER_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        None,
        move || {
            while let Ok(text) = rx.recv() {
                queued_bytes.fetch_sub(text.len(), Ordering::Relaxed);
                if let Err(err) = sink.send_text(&text) {
                    log::info!("ws: client {id} send failed ({err:?}); closing");
                    break;
                }
            }
            for text in rx.try_iter() {
                queued_bytes.fetch_sub(text.len(), Ordering::Relaxed);
            }
            drop(rx);
            closed.store(true, Ordering::Relaxed);
            sink.close();
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::sink::WsSinkError;
    use dali2rust_test_support::wait_until;
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::Condvar;
    use std::time::Duration;

    const OVERFILL_BROADCASTS: usize = 8;

    struct RecordingSink {
        tx: Mutex<Sender<String>>,
        gate: Arc<Mutex<bool>>,
    }

    impl WsSink for RecordingSink {
        fn send_text(&self, text: &str) -> Result<(), WsSinkError> {
            if *self.gate.lock().unwrap() {
                return Err(WsSinkError::Closed);
            }
            self.tx
                .lock()
                .unwrap()
                .send(text.to_string())
                .map_err(|_| WsSinkError::Closed)
        }
        fn close(&self) {}
    }

    fn hub() -> (Arc<WsHub>, Arc<SnifferTap>) {
        let (tap, _rx) = SnifferTap::new(8);
        let hub = WsHub::new(
            WsHubConfig::default(),
            Arc::new(WsCounters::default()),
            Arc::clone(&tap),
            leaked_test_ring(),
        );
        (hub, tap)
    }

    fn connect(hub: &WsHub) -> (ClientId, Receiver<String>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let sink = RecordingSink {
            tx: Mutex::new(tx),
            gate: Arc::new(Mutex::new(false)),
        };
        let id = hub.register(Box::new(sink)).expect("register");
        (id, rx)
    }

    fn next(rx: &Receiver<String>) -> String {
        rx.recv_timeout(std::time::Duration::from_secs(2))
            .expect("frame")
    }

    #[test]
    fn the_subscription_mask_matches_the_clients_after_every_mutation() {
        let (hub, _tap) = hub();
        let expected = |hub: &WsHub| {
            hub.lock()
                .iter()
                .fold(0u16, |mask, c| mask | c.subscriptions)
        };
        let check = |hub: &WsHub, label: &str| {
            assert_eq!(
                hub.subscriptions.load(Ordering::Relaxed),
                expected(hub),
                "{label}"
            );
        };

        check(&hub, "empty hub");

        let (a, rx_a) = connect(&hub);
        let _hello = next(&rx_a);
        check(&hub, "after register, before any subscribe");

        hub.handle_client_text(a, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);
        let _ack = next(&rx_a);
        check(&hub, "after subscribe");
        assert!(hub.any_subscriber(Channel::VirtualLamps));

        let (b, rx_b) = connect(&hub);
        let _hello_b = next(&rx_b);
        hub.handle_client_text(b, r#"{"op":"subscribe","channels":["groups"]}"#);
        let _ack_b = next(&rx_b);
        check(&hub, "two clients, disjoint channels");
        assert!(hub.any_subscriber(Channel::Groups));

        hub.handle_client_text(a, r#"{"op":"unsubscribe","channels":["virtual_lamps"]}"#);
        let _ack = next(&rx_a);
        check(&hub, "after unsubscribe");
        assert!(!hub.any_subscriber(Channel::VirtualLamps));

        hub.unregister(b);
        check(&hub, "after unregister");
        assert!(
            !hub.any_subscriber(Channel::Groups),
            "the only groups subscriber left"
        );

        let (doomed_tx, _doomed_rx) = std::sync::mpsc::channel();
        let doomed = hub
            .register(Box::new(RecordingSink {
                tx: Mutex::new(doomed_tx),
                gate: Arc::new(Mutex::new(true)),
            }))
            .expect("register");
        hub.handle_client_text(doomed, r#"{"op":"subscribe","channels":["sniffer"]}"#);
        assert!(hub.any_subscriber(Channel::Sniffer));
        wait_until(
            || {
                hub.reap_closed();
                hub.client_count() == 1
            },
            Duration::from_secs(2),
        );
        check(&hub, "after reap");
        assert!(
            !hub.any_subscriber(Channel::Sniffer),
            "a reaped client must not keep its channel alive"
        );
    }

    #[test]
    fn an_upgraded_client_is_greeted_before_anything_else() {
        let (hub, _tap) = hub();
        let (_id, rx) = connect(&hub);
        assert!(next(&rx).contains("\"op\":\"hello\""));
    }

    #[test]
    fn the_ring_stays_armed_until_somebody_asks_for_more() {
        let (hub, _tap) = hub();
        let (id, rx) = connect(&hub);
        let _hello = next(&rx);
        assert_eq!(hub.log_ring().min_level(), ARMED_LEVEL as u8);

        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);
        let _ack = next(&rx);
        assert_eq!(
            hub.log_ring().min_level(),
            ARMED_LEVEL as u8,
            "a subscription to another channel must not open the log level"
        );
    }

    #[test]
    fn the_loudest_subscriber_sets_the_level_and_the_last_to_leave_restores_it() {
        let (hub, _tap) = hub();
        let (quiet, quiet_rx) = connect(&hub);
        let (loud, loud_rx) = connect(&hub);
        let _hello = (next(&quiet_rx), next(&loud_rx));

        hub.handle_client_text(quiet, r#"{"op":"subscribe","channels":["logs"],"logs":{"min_level":"warn"}}"#);
        let _ack = next(&quiet_rx);
        assert_eq!(hub.log_ring().min_level(), LogLevel::Warn as u8);

        hub.handle_client_text(loud, r#"{"op":"subscribe","channels":["logs"],"logs":{"min_level":"debug"}}"#);
        let _ack = next(&loud_rx);
        assert_eq!(hub.log_ring().min_level(), LogLevel::Debug as u8);

        hub.handle_client_text(loud, r#"{"op":"unsubscribe","channels":["logs"]}"#);
        let _ack = next(&loud_rx);
        assert_eq!(
            hub.log_ring().min_level(),
            LogLevel::Warn as u8,
            "the quiet subscriber is still watching"
        );

        hub.handle_client_text(quiet, r#"{"op":"unsubscribe","channels":["logs"]}"#);
        let _ack = next(&quiet_rx);
        assert_eq!(hub.log_ring().min_level(), ARMED_LEVEL as u8);
    }

    #[test]
    fn a_logs_subscription_without_a_level_opens_the_ring_to_info() {
        let (hub, _tap) = hub();
        let (id, rx) = connect(&hub);
        let _hello = next(&rx);

        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["logs"]}"#);
        let _ack = next(&rx);
        assert_eq!(hub.log_ring().min_level(), LogLevel::Info as u8);
    }

    #[test]
    fn subscribing_to_logs_asks_the_ring_to_rewind() {
        let (hub, _tap) = hub();
        let (id, rx) = connect(&hub);
        let _hello = next(&rx);
        assert_eq!(hub.log_ring().take_replay(), None);

        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["logs"]}"#);
        let _ack = next(&rx);
        assert!(
            hub.log_ring().take_replay().is_some(),
            "a subscribe must leave a rewind for the worker"
        );
    }

    #[test]
    fn subscribe_merges_and_acks_the_full_acting_set() {
        let (hub, _tap) = hub();
        let (id, rx) = connect(&hub);
        let _hello = next(&rx);
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);
        assert!(next(&rx).contains("virtual_lamps"));
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["groups"]}"#);
        let ack = next(&rx);
        assert!(ack.contains("virtual_lamps"), "{ack}");
        assert!(ack.contains("groups"), "{ack}");
    }

    #[test]
    fn a_rejected_subscribe_leaves_the_acting_set_untouched() {
        let (hub, _tap) = hub();
        let (id, rx) = connect(&hub);
        let _hello = next(&rx);
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);
        let _ack = next(&rx);
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["groups","nope"]}"#);
        assert!(next(&rx).contains("invalid_channel"));
        assert!(hub.any_subscriber(Channel::VirtualLamps));
        assert!(!hub.any_subscriber(Channel::Groups));
    }

    #[test]
    fn duplicate_subscribe_delivers_each_event_once() {
        let (hub, _tap) = hub();
        let (id, rx) = connect(&hub);
        let _hello = next(&rx);
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);
        let _ = next(&rx);
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);
        let _ = next(&rx);
        hub.broadcast(Channel::VirtualLamps, "EVENT");
        assert_eq!(next(&rx), "EVENT");
        assert!(rx
            .recv_timeout(std::time::Duration::from_millis(150))
            .is_err());
    }

    #[test]
    fn events_only_reach_subscribers_of_that_channel() {
        let (hub, _tap) = hub();
        let (id, rx) = connect(&hub);
        let _hello = next(&rx);
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["groups"]}"#);
        let _ = next(&rx);
        hub.broadcast(Channel::Scenes, "SCENE");
        hub.broadcast(Channel::Groups, "GROUP");
        assert_eq!(next(&rx), "GROUP");
    }

    #[test]
    fn the_client_cap_is_enforced_and_counted() {
        let (hub, _tap) = hub();
        let mut keep = Vec::new();
        for _ in 0..MAX_WS_CLIENTS {
            keep.push(connect(&hub));
        }
        let (tx, _rx) = std::sync::mpsc::channel();
        let over = hub.register(Box::new(RecordingSink {
            tx: Mutex::new(tx),
            gate: Arc::new(Mutex::new(false)),
        }));
        assert_eq!(over, Err(RegisterRejected::CapacityExhausted));
        assert_eq!(
            WsCounters::load(&hub.counters().upgrades_rejected_total),
            1
        );
        assert!(WsHub::rejection_frame(RegisterRejected::CapacityExhausted)
            .contains("ws_clients_exhausted"));
    }

    #[test]
    fn the_sniffer_tap_runs_only_while_somebody_is_watching() {
        let (hub, tap) = hub();
        let (id, rx) = connect(&hub);
        let _hello = next(&rx);
        assert!(!tap.is_enabled());
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["sniffer"]}"#);
        let _ = next(&rx);
        assert!(tap.is_enabled());
        hub.handle_client_text(id, r#"{"op":"unsubscribe","channels":["sniffer"]}"#);
        let _ = next(&rx);
        assert!(!tap.is_enabled());
    }

    #[test]
    fn disconnecting_the_last_watcher_switches_the_tap_off() {
        let (hub, tap) = hub();
        let (id, rx) = connect(&hub);
        let _hello = next(&rx);
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["sniffer"]}"#);
        let _ = next(&rx);
        assert!(tap.is_enabled());
        hub.unregister(id);
        assert!(!tap.is_enabled());
        assert_eq!(hub.client_count(), 0);
    }

    #[test]
    fn a_reaped_watcher_never_switches_the_tap_off_under_a_live_one() {
        let (hub, tap) = hub();
        let (doomed_tx, _doomed_rx) = std::sync::mpsc::channel();
        let doomed = hub
            .register(Box::new(RecordingSink {
                tx: Mutex::new(doomed_tx),
                gate: Arc::new(Mutex::new(true)),
            }))
            .expect("register");
        hub.handle_client_text(doomed, r#"{"op":"subscribe","channels":["sniffer"]}"#);
        let (fresh, rx_fresh) = connect(&hub);
        let _hello = next(&rx_fresh);
        hub.handle_client_text(fresh, r#"{"op":"subscribe","channels":["sniffer"]}"#);
        let _ = next(&rx_fresh);
        assert!(tap.is_enabled());
        wait_until(
            || {
                hub.reap_closed();
                hub.client_count() == 1
            },
            Duration::from_secs(2),
        );
        assert!(
            tap.is_enabled(),
            "the reap ran after the subscribe and must not undo it"
        );
        hub.unregister(fresh);
        assert!(!tap.is_enabled());
    }

    fn blocking_hub() -> Arc<WsHub> {
        let (tap, _rx) = SnifferTap::new(8);
        WsHub::new(
            WsHubConfig {
                max_clients: 2,
                queue_depth: 2,
            },
            Arc::new(WsCounters::default()),
            tap,
            leaked_test_ring(),
        )
    }

    #[test]
    fn a_full_client_queue_drops_and_then_reports_what_it_lost() {
        let hub = blocking_hub();
        let (blocked_tx, _blocked_rx) = std::sync::mpsc::channel::<String>();
        let gate = Arc::new(Mutex::new(false));
        let held = gate.lock().unwrap();
        let id = hub
            .register(Box::new(RecordingSink {
                tx: Mutex::new(blocked_tx),
                gate: Arc::clone(&gate),
            }))
            .expect("register");
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);
        for _ in 0..64 {
            hub.broadcast(Channel::VirtualLamps, "EVENT");
        }
        assert!(
            WsCounters::load(&hub.counters().events_dropped_total) > 0,
            "a full queue must count its losses"
        );
        hub.reap_closed();
        assert_eq!(hub.client_count(), 1, "a slow client is not a gone client");
        drop(held);
    }

    #[test]
    fn a_client_is_shed_on_bytes_long_before_it_reaches_the_frame_cap() {
        let (tap, _rx) = SnifferTap::new(8);
        let hub = WsHub::new(
            WsHubConfig {
                max_clients: 2,
                queue_depth: 32,
            },
            Arc::new(WsCounters::default()),
            tap,
            leaked_test_ring(),
        );
        let (blocked_tx, _blocked_rx) = std::sync::mpsc::channel::<String>();
        let gate = Arc::new(Mutex::new(false));
        let held = gate.lock().unwrap();
        let id = hub
            .register(Box::new(RecordingSink {
                tx: Mutex::new(blocked_tx),
                gate: Arc::clone(&gate),
            }))
            .expect("register");
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);

        let fat = "x".repeat(4096);
        for _ in 0..32 {
            hub.broadcast(Channel::VirtualLamps, &fat);
        }
        let sent = WsCounters::load(&hub.counters().events_sent_total);
        let dropped = WsCounters::load(&hub.counters().events_dropped_total);

        assert!(dropped > 0, "a 128 KiB burst must be shed somewhere");
        assert!(
            sent <= 8,
            "bytes must bind before the frame cap: {sent} accepted of 32"
        );
        assert_eq!(
            sent + dropped,
            32,
            "every frame is either sent or counted, never neither"
        );
        drop(held);
    }

    #[test]
    fn a_client_whose_thread_has_gone_is_not_counted_as_a_drop() {
        let hub = blocking_hub();
        let (blocked_tx, blocked_rx) = std::sync::mpsc::channel::<String>();
        let id = hub
            .register(Box::new(RecordingSink {
                tx: Mutex::new(blocked_tx),
                gate: Arc::new(Mutex::new(true)),
            }))
            .expect("register");
        drop(blocked_rx);
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["virtual_lamps"]}"#);
        wait_until(
            || {
                hub.broadcast(Channel::VirtualLamps, "EVENT");
                hub.reap_closed();
                hub.client_count() == 0
            },
            Duration::from_secs(2),
        );
        assert_eq!(hub.client_count(), 0, "the dead client must be reaped");
        let after_reap = WsCounters::load(&hub.counters().events_dropped_total);
        for _ in 0..64 {
            hub.broadcast(Channel::VirtualLamps, "EVENT");
        }
        assert_eq!(
            WsCounters::load(&hub.counters().events_dropped_total),
            after_reap,
            "a socket nobody holds cannot fall behind"
        );
    }

    #[test]
    fn origin_is_checked_only_when_the_client_states_one() {
        assert!(origin_allowed(None, Some("192.168.11.41")));
        assert!(origin_allowed(None, None));
        assert!(origin_allowed(
            Some("http://192.168.11.41"),
            Some("192.168.11.41")
        ));
        assert!(origin_allowed(
            Some("https://dali.local:8443"),
            Some("dali.local:8443")
        ));
        assert!(origin_allowed(Some("http://DALI.local"), Some("dali.local")));
        assert!(!origin_allowed(Some("http://evil.example"), Some("dali.local")));
        assert!(!origin_allowed(Some("null"), Some("dali.local")));
        assert!(!origin_allowed(Some("http://dali.local"), None));
        assert!(!origin_allowed(
            Some("http://evil//dali.local"),
            Some("dali.local")
        ));
        assert!(!origin_allowed(
            Some("http://evil.example/dali.local"),
            Some("dali.local")
        ));
        assert!(!origin_allowed(Some("dali.local"), Some("dali.local")));
    }

    #[test]
    fn each_refusal_closes_with_its_own_code() {
        assert_eq!(close_code(RegisterRejected::CapacityExhausted), 1013);
        assert_eq!(close_code(RegisterRejected::OriginRejected), 1008);
    }

    #[test]
    fn a_refused_origin_says_so_on_the_protocol() {
        let frame = WsHub::rejection_frame(RegisterRejected::OriginRejected);
        assert!(frame.contains("ws_origin_rejected"), "{frame}");
    }

    #[test]
    fn inbox_overflow_tells_every_client_its_whole_view_is_stale() {
        let (hub, _tap) = hub();
        let (id_a, rx_a) = connect(&hub);
        let (id_b, rx_b) = connect(&hub);
        let _ = (next(&rx_a), next(&rx_b));
        hub.handle_client_text(id_a, r#"{"op":"subscribe","channels":["groups"]}"#);
        hub.handle_client_text(id_b, r#"{"op":"subscribe","channels":["scenes"]}"#);
        let _ = (next(&rx_a), next(&rx_b));
        hub.note_inbox_overflow(5);
        for rx in [&rx_a, &rx_b] {
            let frame = next(rx);
            assert!(frame.contains("DropNotice"), "{frame}");
            assert!(frame.contains("\"channel\":\"*\""), "{frame}");
            assert!(frame.contains("\"dropped_count\":5"), "{frame}");
        }
        assert_eq!(WsCounters::load(&hub.counters().inbox_overflow_total), 5);
    }

    struct PausedSink {
        frames: Arc<Mutex<Vec<String>>>,
        hold: Arc<(Mutex<bool>, Condvar)>,
    }

    impl WsSink for PausedSink {
        fn send_text(&self, text: &str) -> Result<(), WsSinkError> {
            let (lock, released) = &*self.hold;
            let mut held = lock.lock().unwrap();
            while *held {
                held = released.wait(held).unwrap();
            }
            drop(held);
            self.frames.lock().unwrap().push(text.to_string());
            Ok(())
        }
        fn close(&self) {}
    }

    fn find(frames: &Arc<Mutex<Vec<String>>>, needle: &str) -> Option<String> {
        frames
            .lock()
            .unwrap()
            .iter()
            .find(|f| f.contains(needle))
            .cloned()
    }

    #[test]
    fn an_inbox_notice_that_did_not_fit_is_retried_on_the_next_delivery() {
        let (tap, _tap_rx) = SnifferTap::new(8);
        let hub = WsHub::new(
            WsHubConfig {
                max_clients: 1,
                queue_depth: 2,
            },
            Arc::new(WsCounters::default()),
            tap,
            leaked_test_ring(),
        );
        let frames = Arc::new(Mutex::new(Vec::new()));
        let hold = Arc::new((Mutex::new(true), Condvar::new()));
        let id = hub
            .register(Box::new(PausedSink {
                frames: Arc::clone(&frames),
                hold: Arc::clone(&hold),
            }))
            .expect("register");
        hub.handle_client_text(id, r#"{"op":"subscribe","channels":["groups"]}"#);
        for _ in 0..OVERFILL_BROADCASTS {
            hub.broadcast(Channel::Groups, "EVENT");
        }
        assert!(
            WsCounters::load(&hub.counters().events_dropped_total) > 0,
            "precondition: the queue must be full, or the notice would have fit"
        );
        hub.note_inbox_overflow(3);
        assert!(
            find(&frames, "\"channel\":\"*\"").is_none(),
            "precondition: nothing can have been written while the sink is held"
        );

        *hold.0.lock().unwrap() = false;
        hold.1.notify_all();
        wait_until(
            || {
                hub.broadcast(Channel::Groups, "EVENT");
                find(&frames, "\"channel\":\"*\"").is_some()
            },
            Duration::from_secs(2),
        );
        let notice = find(&frames, "\"channel\":\"*\"").expect("wait_until returned");
        assert!(
            notice.contains("\"dropped_count\":3"),
            "the retried notice must still carry the whole debt: {notice}"
        );
    }

    #[test]
    fn sweeping_a_reused_fd_releases_the_client_it_named() {
        let (hub, _tap) = hub();
        let sessions = WsSessions::new();
        let mut kept = Vec::new();
        for fd in 0..MAX_WS_CLIENTS as i32 {
            let (id, rx) = connect(&hub);
            sessions.insert(fd, id);
            kept.push(rx);
        }
        assert_eq!(hub.client_count(), MAX_WS_CLIENTS);

        const REUSED_FD: i32 = 0;
        sessions.close(&hub, REUSED_FD);
        assert_eq!(sessions.lookup(REUSED_FD), None);
        assert_eq!(
            hub.client_count(),
            MAX_WS_CLIENTS - 1,
            "a swept row must release the slot, not leave a zombie holding it"
        );

        let (reconnected, _rx) = connect(&hub);
        sessions.insert(REUSED_FD, reconnected);
        assert_eq!(sessions.lookup(REUSED_FD), Some(reconnected));
    }
}
