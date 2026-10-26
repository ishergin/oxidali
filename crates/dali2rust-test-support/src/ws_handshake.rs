use std::net::TcpStream;
use std::time::Duration;

use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::client::Request;
use tungstenite::WebSocket;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

pub fn connect(port: u16, origin: Option<&str>, poll: Duration) -> WebSocket<TcpStream> {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("ws tcp connect");
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .expect("ws handshake timeout");
    let (socket, _response) = tungstenite::client::client(upgrade_request(port, origin), stream)
        .expect("ws handshake");
    socket
        .get_ref()
        .set_read_timeout(Some(poll))
        .expect("ws read timeout");
    socket
}

fn upgrade_request(port: u16, origin: Option<&str>) -> Request {
    let mut request = format!("ws://127.0.0.1:{port}/api/v1/ws")
        .into_client_request()
        .expect("ws request");
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("Origin", origin.parse().expect("origin header"));
    }
    request
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};

    use tungstenite::handshake::server::{Callback, ErrorResponse, Request, Response};

    use super::*;

    const POLL: Duration = Duration::from_millis(100);
    const SLOW_UPGRADE: Duration = Duration::from_millis(150);

    struct AnswerAfter(Duration);

    impl Callback for AnswerAfter {
        fn on_request(
            self,
            _request: &Request,
            response: Response,
        ) -> Result<Response, ErrorResponse> {
            // sleep-ok: the fake server answers the upgrade slower than POLL
            thread::sleep(self.0);
            Ok(response)
        }
    }

    fn serve_one_upgrade(delay: Duration) -> (u16, JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("local addr").port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            tungstenite::accept_hdr(stream, AnswerAfter(delay)).expect("server handshake");
        });
        (port, server)
    }

    #[test]
    fn a_handshake_slower_than_the_poll_timeout_still_connects() {
        assert!(SLOW_UPGRADE > POLL);
        let (port, server) = serve_one_upgrade(SLOW_UPGRADE);
        let _socket = connect(port, None, POLL);
        server.join().expect("server thread");
    }

    #[test]
    fn the_socket_polls_under_its_own_timeout_once_connected() {
        let (port, server) = serve_one_upgrade(Duration::ZERO);
        let socket = connect(port, None, POLL);
        server.join().expect("server thread");
        let polling = socket.get_ref().read_timeout().expect("read timeout");
        assert_eq!(polling, Some(POLL));
    }
}
