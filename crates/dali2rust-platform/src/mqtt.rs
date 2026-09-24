use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MqttQos {
    AtMostOnce,
    AtLeastOnce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MqttConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

impl MqttConnectionState {
    fn as_u8(self) -> u8 {
        match self {
            Self::Disconnected => 0,
            Self::Connecting => 1,
            Self::Connected => 2,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Connecting,
            2 => Self::Connected,
            _ => Self::Disconnected,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttLastWill {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: MqttQos,
    pub retain: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttSessionConfig {
    pub broker_host: String,
    pub broker_port: u16,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub keep_alive: Duration,
    pub last_will: Option<MqttLastWill>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MqttIncoming {
    pub topic: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MqttError {
    NotConnected,
    PayloadTooLarge,
    Rejected(i32),
    ConfigInvalid,
}

pub const INCOMING_QUEUE_DEPTH: usize = 16;

#[derive(Debug)]
pub struct MqttLink {
    state: AtomicU8,
    session_generation: AtomicU32,
    subscriptions_acked: AtomicU32,
    dropped_incoming: AtomicU32,
    tx: std::sync::mpsc::SyncSender<MqttIncoming>,
    changed: Condvar,
    changed_guard: Mutex<()>,
}

impl MqttLink {
    pub fn new() -> (std::sync::Arc<Self>, std::sync::mpsc::Receiver<MqttIncoming>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(INCOMING_QUEUE_DEPTH);
        let link = std::sync::Arc::new(Self {
            state: AtomicU8::new(MqttConnectionState::Disconnected.as_u8()),
            session_generation: AtomicU32::new(0),
            subscriptions_acked: AtomicU32::new(0),
            dropped_incoming: AtomicU32::new(0),
            tx,
            changed: Condvar::new(),
            changed_guard: Mutex::new(()),
        });
        (link, rx)
    }

    pub fn state(&self) -> MqttConnectionState {
        MqttConnectionState::from_u8(self.state.load(Ordering::Relaxed))
    }

    pub fn is_connected(&self) -> bool {
        self.state() == MqttConnectionState::Connected
    }

    pub fn session_generation(&self) -> u32 {
        self.session_generation.load(Ordering::Relaxed)
    }

    pub fn dropped_incoming(&self) -> u32 {
        self.dropped_incoming.load(Ordering::Relaxed)
    }

    pub fn subscriptions_acked(&self) -> u32 {
        self.subscriptions_acked.load(Ordering::Relaxed)
    }

    pub fn note_subscription_acked(&self) {
        self.subscriptions_acked.fetch_add(1, Ordering::Relaxed);
        self.wake();
    }

    pub fn set_state(&self, next: MqttConnectionState) {
        if next == MqttConnectionState::Connected
            && self.state() != MqttConnectionState::Connected
        {
            self.session_generation.fetch_add(1, Ordering::Relaxed);
            self.subscriptions_acked.store(0, Ordering::Relaxed);
        }
        self.state.store(next.as_u8(), Ordering::Relaxed);
        self.wake();
    }

    pub fn deliver(&self, message: MqttIncoming) {
        if self.tx.try_send(message).is_err() {
            self.dropped_incoming.fetch_add(1, Ordering::Relaxed);
        }
        self.wake();
    }

    pub fn wake(&self) {
        let _guard = self.changed_guard.lock();
        self.changed.notify_all();
    }

    pub fn wait_for_change(&self, timeout: Duration) {
        let Ok(guard) = self.changed_guard.lock() else {
            return;
        };
        let _ = self.changed.wait_timeout(guard, timeout);
    }
}

pub struct MqttClientBundle {
    pub client: Box<dyn MqttClient>,
    pub incoming: std::sync::mpsc::Receiver<MqttIncoming>,
}

pub trait MqttClient: Send {
    fn connect(&mut self, config: &MqttSessionConfig) -> Result<(), MqttError>;

    fn disconnect(&mut self);

    fn publish(
        &mut self,
        topic: &str,
        payload: &[u8],
        qos: MqttQos,
        retain: bool,
    ) -> Result<(), MqttError>;

    fn subscribe(&mut self, topic_filter: &str, qos: MqttQos) -> Result<(), MqttError>;

    fn link(&self) -> std::sync::Arc<MqttLink>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reconnect_is_visible_as_a_new_generation_not_just_a_state() {
        let (link, _rx) = MqttLink::new();
        assert_eq!(link.session_generation(), 0);
        link.set_state(MqttConnectionState::Connected);
        assert_eq!(link.session_generation(), 1);
        link.set_state(MqttConnectionState::Connected);
        assert_eq!(link.session_generation(), 1);
        link.set_state(MqttConnectionState::Disconnected);
        link.set_state(MqttConnectionState::Connected);
        assert_eq!(link.session_generation(), 2);
    }

    #[test]
    fn a_worker_that_is_behind_loses_messages_with_a_count_rather_than_blocking() {
        let (link, rx) = MqttLink::new();
        for i in 0..INCOMING_QUEUE_DEPTH + 3 {
            link.deliver(MqttIncoming {
                topic: format!("t/{i}"),
                payload: Vec::new(),
            });
        }
        assert_eq!(link.dropped_incoming(), 3);
        assert_eq!(rx.iter().take(INCOMING_QUEUE_DEPTH).count(), INCOMING_QUEUE_DEPTH);
    }

    #[test]
    fn waiting_returns_on_a_timeout_so_a_lost_wake_cannot_park_the_worker_forever() {
        let (link, _rx) = MqttLink::new();
        let started = std::time::Instant::now();
        link.wait_for_change(Duration::from_millis(20));
        assert!(started.elapsed() >= Duration::from_millis(15));
    }
}
