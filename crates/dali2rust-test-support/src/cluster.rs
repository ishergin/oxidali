use std::io::ErrorKind;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dali2rust_platform::cluster_link::{
    ClusterLink, ClusterLinkBinder, ClusterLinkError, ClusterReceiveBuffer, Received,
    MAX_CLUSTER_DATAGRAM_BYTES,
};

const HUB_SUBNET: [u8; 3] = [10, 0, 0];
const FIRST_HUB_HOST: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubDatagram {
    pub source: [u8; 4],
    pub bytes: Vec<u8>,
    pub at: Instant,
}

struct EndQueue {
    source: [u8; 4],
    inbox: Sender<HubDatagram>,
}

type Loss = Box<dyn FnMut(&HubDatagram) -> bool + Send>;

struct HubState {
    ends: Vec<EndQueue>,
    sent: Sender<HubDatagram>,
    next_host: u8,
    refusing: bool,
    loss: Option<Loss>,
}

impl HubState {
    fn deliver(&mut self, datagram: &HubDatagram) {
        self.ends.retain(|end| {
            end.source == datagram.source || end.inbox.send(datagram.clone()).is_ok()
        });
    }
}

pub struct ClusterHub {
    state: Arc<Mutex<HubState>>,
    sent: Mutex<Receiver<HubDatagram>>,
}

impl Default for ClusterHub {
    fn default() -> Self {
        Self::new()
    }
}

impl ClusterHub {
    pub fn new() -> Self {
        let (sent_tx, sent_rx) = mpsc::channel();
        let state = HubState { ends: Vec::new(), sent: sent_tx, next_host: FIRST_HUB_HOST, refusing: false, loss: None };
        Self { state: Arc::new(Mutex::new(state)), sent: Mutex::new(sent_rx) }
    }

    pub fn attach(&self) -> HubEnd {
        let source = {
            let mut state = lock(&self.state);
            let [a, b, c] = HUB_SUBNET;
            let source = [a, b, c, state.next_host];
            state.next_host = state.next_host.checked_add(1).expect("a hub holds at most 254 ends");
            source
        };
        self.attach_at(source)
    }

    pub fn attach_at(&self, source: [u8; 4]) -> HubEnd {
        let (inbox_tx, inbox_rx) = mpsc::channel();
        lock(&self.state).ends.push(EndQueue { source, inbox: inbox_tx });
        HubEnd { source, inbox: Mutex::new(inbox_rx), state: Arc::clone(&self.state) }
    }

    pub fn inject(&self, source: [u8; 4], bytes: &[u8]) {
        lock(&self.state).deliver(&HubDatagram { source, bytes: bytes.to_vec(), at: Instant::now() });
    }

    pub fn refuse_sends(&self, refusing: bool) {
        lock(&self.state).refusing = refusing;
    }

    pub fn lose_sends(&self, lose: impl FnMut(&HubDatagram) -> bool + Send + 'static) {
        lock(&self.state).loss = Some(Box::new(lose));
    }

    pub fn next_sent(&self, timeout: Duration) -> Option<HubDatagram> {
        lock(&self.sent).recv_timeout(timeout).ok()
    }
}

pub struct HubEnd {
    source: [u8; 4],
    inbox: Mutex<Receiver<HubDatagram>>,
    state: Arc<Mutex<HubState>>,
}

impl HubEnd {
    pub fn source(&self) -> [u8; 4] {
        self.source
    }
}

impl ClusterLink for HubEnd {
    fn send(&self, datagram: &[u8]) -> Result<(), ClusterLinkError> {
        if datagram.len() > MAX_CLUSTER_DATAGRAM_BYTES {
            return Err(ClusterLinkError::TooLarge);
        }
        let datagram = HubDatagram { source: self.source, bytes: datagram.to_vec(), at: Instant::now() };
        let mut state = lock(&self.state);
        if state.refusing {
            return Err(ClusterLinkError::Io(ErrorKind::ConnectionRefused));
        }
        let lost = state.loss.as_mut().is_some_and(|lose| lose(&datagram));
        if !lost {
            state.deliver(&datagram);
        }
        let _ = state.sent.send(datagram);
        Ok(())
    }

    fn receive(
        &self,
        buffer: &mut ClusterReceiveBuffer,
        timeout: Duration,
    ) -> Result<Received, ClusterLinkError> {
        let Ok(datagram) = lock(&self.inbox).recv_timeout(timeout) else {
            return Ok(Received::TimedOut);
        };
        let kept = datagram.bytes.len().min(buffer.len());
        buffer[..kept].copy_from_slice(&datagram.bytes[..kept]);
        Ok(Received::of(datagram.bytes.len(), datagram.source))
    }
}

pub struct ClusterHubBinder {
    hub: Arc<ClusterHub>,
    source: [u8; 4],
}

impl ClusterHubBinder {
    pub fn new(hub: Arc<ClusterHub>, source: [u8; 4]) -> Self {
        Self { hub, source }
    }
}

impl ClusterLinkBinder for ClusterHubBinder {
    fn bind(&self, _port: u16) -> Result<Arc<dyn ClusterLink>, ClusterLinkError> {
        Ok(Arc::new(self.hub.attach_at(self.source)))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use dali2rust_platform::cluster_link::CLUSTER_RECEIVE_BUFFER_BYTES;

    use super::*;

    const PATIENCE: Duration = Duration::from_secs(5);
    const BRIEF_WAIT: Duration = Duration::from_millis(20);
    const NEIGHBOUR: [u8; 4] = [192, 168, 1, 50];
    const PULSE: &[u8] = b"pulse";
    const HELLO: &[u8] = b"hello";

    fn receive(end: &HubEnd) -> (Received, Vec<u8>) {
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        let got = end.receive(&mut buffer, PATIENCE).expect("receive");
        let len = match got {
            Received::Datagram { len, .. } => len,
            _ => 0,
        };
        (got, buffer[..len].to_vec())
    }

    fn assert_silent(end: &HubEnd) {
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        assert_eq!(end.receive(&mut buffer, BRIEF_WAIT), Ok(Received::TimedOut));
    }

    #[test]
    fn a_datagram_reaches_every_other_end_and_not_its_sender() {
        let hub = ClusterHub::new();
        let (a, b, c) = (hub.attach(), hub.attach(), hub.attach());
        a.send(PULSE).expect("send");
        for end in [&b, &c] {
            let (got, bytes) = receive(end);
            assert_eq!(got, Received::Datagram { len: PULSE.len(), source: a.source() });
            assert_eq!(bytes, PULSE);
        }
        assert_silent(&a);
    }

    #[test]
    fn a_lost_datagram_reaches_nobody_and_is_still_seen_as_sent() {
        let hub = ClusterHub::new();
        let (a, b) = (hub.attach(), hub.attach());
        hub.lose_sends(|datagram| datagram.bytes == PULSE);
        a.send(PULSE).expect("a lost datagram still leaves its sender");
        a.send(HELLO).expect("send");
        let (_, bytes) = receive(&b);
        assert_eq!(bytes, HELLO, "the lost datagram never arrived");
        assert_eq!(hub.next_sent(PATIENCE).map(|sent| sent.bytes), Some(PULSE.to_vec()));
    }

    #[test]
    fn every_end_has_its_own_address() {
        let hub = ClusterHub::new();
        let (a, b) = (hub.attach(), hub.attach());
        assert_ne!(a.source(), b.source());
    }

    #[test]
    fn an_injected_datagram_reaches_every_end_from_the_address_the_test_names() {
        let hub = ClusterHub::new();
        let (a, b) = (hub.attach(), hub.attach());
        hub.inject(NEIGHBOUR, HELLO);
        for end in [&a, &b] {
            let (got, bytes) = receive(end);
            assert_eq!(got, Received::Datagram { len: HELLO.len(), source: NEIGHBOUR });
            assert_eq!(bytes, HELLO);
        }
        assert_eq!(hub.next_sent(BRIEF_WAIT), None);
    }

    #[test]
    fn the_test_sees_what_each_end_sent_in_order() {
        let hub = ClusterHub::new();
        let (a, b) = (hub.attach(), hub.attach());
        a.send(b"first").expect("send");
        b.send(b"second").expect("send");
        let first = hub.next_sent(PATIENCE).expect("first");
        let second = hub.next_sent(PATIENCE).expect("second");
        assert_eq!((first.source, first.bytes), (a.source(), b"first".to_vec()));
        assert_eq!((second.source, second.bytes), (b.source(), b"second".to_vec()));
    }

    #[test]
    fn a_datagram_past_the_ceiling_is_refused_like_the_socket_refuses_it() {
        let hub = ClusterHub::new();
        let (a, b) = (hub.attach(), hub.attach());
        assert_eq!(a.send(&[0; MAX_CLUSTER_DATAGRAM_BYTES + 1]), Err(ClusterLinkError::TooLarge));
        assert_silent(&b);
        assert_eq!(hub.next_sent(BRIEF_WAIT), None);
    }

    #[test]
    fn an_injected_datagram_past_the_ceiling_arrives_as_oversize() {
        let hub = ClusterHub::new();
        let a = hub.attach();
        hub.inject(NEIGHBOUR, &[0; MAX_CLUSTER_DATAGRAM_BYTES * 2]);
        assert_eq!(receive(&a).0, Received::Oversize { source: NEIGHBOUR });
    }

    #[test]
    fn a_binder_attaches_every_bind_at_the_address_it_was_given() {
        let hub = Arc::new(ClusterHub::new());
        let watcher = hub.attach();
        let binder = ClusterHubBinder::new(Arc::clone(&hub), NEIGHBOUR);
        let first = binder.bind(1).expect("bind");
        drop(first);
        let second = binder.bind(2).expect("bind");
        second.send(PULSE).expect("send");
        assert_eq!(receive(&watcher).0, Received::Datagram { len: PULSE.len(), source: NEIGHBOUR });
    }

    #[test]
    fn a_dropped_end_leaves_the_others_connected() {
        let hub = ClusterHub::new();
        let (a, b, c) = (hub.attach(), hub.attach(), hub.attach());
        drop(b);
        a.send(b"still").expect("send");
        assert_eq!(receive(&c).1, b"still");
    }
}
