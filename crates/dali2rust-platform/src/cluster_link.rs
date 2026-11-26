use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::Duration;

pub const MAX_CLUSTER_DATAGRAM_BYTES: usize = 1472;

pub const CLUSTER_RECEIVE_BUFFER_BYTES: usize = MAX_CLUSTER_DATAGRAM_BYTES + 1;

pub type ClusterReceiveBuffer = [u8; CLUSTER_RECEIVE_BUFFER_BYTES];

const SHORTEST_RECEIVE_WAIT: Duration = Duration::from_millis(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Received {
    Datagram { len: usize, source: [u8; 4] },
    Oversize { source: [u8; 4] },
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterLinkError {
    TooLarge,
    Io(ErrorKind),
}

pub trait ClusterLink: Send + Sync {
    fn send(&self, datagram: &[u8]) -> Result<(), ClusterLinkError>;

    fn receive(
        &self,
        buffer: &mut ClusterReceiveBuffer,
        timeout: Duration,
    ) -> Result<Received, ClusterLinkError>;
}

pub struct UdpClusterLink {
    socket: UdpSocket,
    destination: SocketAddrV4,
}

impl UdpClusterLink {
    pub fn broadcast(port: u16) -> io::Result<Self> {
        Self::bind(
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port),
            SocketAddrV4::new(Ipv4Addr::BROADCAST, port),
        )
    }

    pub fn bind(local: SocketAddrV4, destination: SocketAddrV4) -> io::Result<Self> {
        let socket = UdpSocket::bind(local)?;
        socket.set_broadcast(true)?;
        Ok(Self { socket, destination })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

impl ClusterLink for UdpClusterLink {
    fn send(&self, datagram: &[u8]) -> Result<(), ClusterLinkError> {
        if datagram.len() > MAX_CLUSTER_DATAGRAM_BYTES {
            return Err(ClusterLinkError::TooLarge);
        }
        self.socket
            .send_to(datagram, self.destination)
            .map(|_| ())
            .map_err(|err| ClusterLinkError::Io(err.kind()))
    }

    fn receive(
        &self,
        buffer: &mut ClusterReceiveBuffer,
        timeout: Duration,
    ) -> Result<Received, ClusterLinkError> {
        self.socket
            .set_read_timeout(Some(timeout.max(SHORTEST_RECEIVE_WAIT)))
            .map_err(|err| ClusterLinkError::Io(err.kind()))?;
        match self.socket.recv_from(buffer) {
            Ok((len, source)) => Ok(Received::of(len, ipv4_of(source))),
            Err(err) if nothing_arrived(err.kind()) => Ok(Received::TimedOut),
            Err(err) => Err(ClusterLinkError::Io(err.kind())),
        }
    }
}

impl Received {
    pub fn of(len: usize, source: [u8; 4]) -> Self {
        if len > MAX_CLUSTER_DATAGRAM_BYTES {
            Self::Oversize { source }
        } else {
            Self::Datagram { len, source }
        }
    }
}

fn nothing_arrived(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

fn ipv4_of(source: SocketAddr) -> [u8; 4] {
    match source {
        SocketAddr::V4(v4) => v4.ip().octets(),
        SocketAddr::V6(v6) => v6
            .ip()
            .to_ipv4_mapped()
            .unwrap_or(Ipv4Addr::UNSPECIFIED)
            .octets(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    const LOOPBACK_ANY_PORT: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);
    const PATIENCE: Duration = Duration::from_secs(5);
    const BRIEF_WAIT: Duration = Duration::from_millis(30);
    const FRAME: &[u8] = b"cluster frame";
    const LOOPBACK: [u8; 4] = Ipv4Addr::LOCALHOST.octets();

    fn loopback_pair() -> (UdpClusterLink, UdpClusterLink) {
        let receiver = UdpClusterLink::bind(LOOPBACK_ANY_PORT, LOOPBACK_ANY_PORT).expect("bind");
        let target = match receiver.local_addr().expect("addr") {
            SocketAddr::V4(v4) => v4,
            SocketAddr::V6(_) => panic!("loopback is v4"),
        };
        let sender = UdpClusterLink::bind(LOOPBACK_ANY_PORT, target).expect("bind");
        (sender, receiver)
    }

    fn receive_within(link: &UdpClusterLink, buffer: &mut ClusterReceiveBuffer) -> Received {
        link.receive(buffer, PATIENCE).expect("receive")
    }

    #[test]
    fn a_datagram_arrives_whole_with_the_address_it_came_from() {
        let (sender, receiver) = loopback_pair();
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        sender.send(FRAME).expect("send");
        let got = receive_within(&receiver, &mut buffer);
        assert_eq!(got, Received::Datagram { len: FRAME.len(), source: LOOPBACK });
        assert_eq!(&buffer[..FRAME.len()], FRAME);
    }

    #[test]
    fn a_datagram_of_exactly_the_ceiling_is_sent_and_received() {
        let (sender, receiver) = loopback_pair();
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        sender.send(&[7; MAX_CLUSTER_DATAGRAM_BYTES]).expect("send");
        let got = receive_within(&receiver, &mut buffer);
        assert_eq!(
            got,
            Received::Datagram { len: MAX_CLUSTER_DATAGRAM_BYTES, source: LOOPBACK }
        );
    }

    #[test]
    fn a_datagram_past_the_ceiling_is_refused_before_the_socket() {
        let (sender, receiver) = loopback_pair();
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        let refused = sender.send(&[7; CLUSTER_RECEIVE_BUFFER_BYTES]);
        assert_eq!(refused, Err(ClusterLinkError::TooLarge));
        assert_eq!(receiver.receive(&mut buffer, BRIEF_WAIT), Ok(Received::TimedOut));
    }

    #[test]
    fn a_datagram_past_the_ceiling_from_elsewhere_is_oversize_never_a_truncated_frame() {
        let (_, receiver) = loopback_pair();
        let foreign = UdpSocket::bind(LOOPBACK_ANY_PORT).expect("bind");
        let target = receiver.local_addr().expect("addr");
        foreign.send_to(&[7; MAX_CLUSTER_DATAGRAM_BYTES * 2], target).expect("send");
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        let got = receive_within(&receiver, &mut buffer);
        assert_eq!(got, Received::Oversize { source: LOOPBACK });
    }

    #[test]
    fn silence_is_a_timeout_and_the_wait_is_bounded() {
        let (_, receiver) = loopback_pair();
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        let started = Instant::now();
        assert_eq!(receiver.receive(&mut buffer, BRIEF_WAIT), Ok(Received::TimedOut));
        assert!(started.elapsed() < PATIENCE);
    }

    #[test]
    fn a_zero_wait_still_returns_rather_than_failing_the_socket_option() {
        let (_, receiver) = loopback_pair();
        let mut buffer = [0; CLUSTER_RECEIVE_BUFFER_BYTES];
        assert_eq!(receiver.receive(&mut buffer, Duration::ZERO), Ok(Received::TimedOut));
    }

    #[test]
    fn the_socket_may_send_to_the_broadcast_address() {
        let (sender, _) = loopback_pair();
        assert!(sender.socket.broadcast().expect("option"));
    }

    #[test]
    fn the_broadcast_link_aims_at_the_limited_broadcast_address_on_its_port() {
        let link = UdpClusterLink::broadcast(0).expect("bind");
        assert_eq!(*link.destination.ip(), Ipv4Addr::BROADCAST);
        assert_eq!(link.local_addr().expect("addr").ip(), Ipv4Addr::UNSPECIFIED);
    }
}
