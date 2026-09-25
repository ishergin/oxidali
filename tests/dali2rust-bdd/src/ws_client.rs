use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::Value;
use tungstenite::{client::IntoClientRequest, Message, WebSocket};

pub const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

const READ_TIMEOUT: Duration = Duration::from_millis(100);

pub struct WsTestClient {
    socket: WebSocket<TcpStream>,
    pending: Vec<Value>,
    reading: bool,
    closed: bool,
    close_code: Option<u16>,
    pongs: u32,
}

impl std::fmt::Debug for WsTestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsTestClient")
            .field("pending", &self.pending.len())
            .field("reading", &self.reading)
            .field("closed", &self.closed)
            .finish()
    }
}

impl WsTestClient {
    pub fn connect(port: u16) -> Self {
        Self::connect_as(port, None)
    }

    pub fn connect_with_origin(port: u16, origin: &str) -> Self {
        Self::connect_as(port, Some(origin))
    }

    fn connect_as(port: u16, origin: Option<&str>) -> Self {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("ws tcp connect");
        stream
            .set_read_timeout(Some(READ_TIMEOUT))
            .expect("ws read timeout");
        let mut request = format!("ws://127.0.0.1:{port}/api/v1/ws")
            .into_client_request()
            .expect("ws request");
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("Origin", origin.parse().expect("origin header"));
        }
        let (socket, _response) =
            tungstenite::client::client(request, stream).expect("ws handshake");
        Self {
            socket,
            pending: Vec::new(),
            reading: true,
            closed: false,
            close_code: None,
            pongs: 0,
        }
    }

    pub fn wait_for_close_code(&mut self) -> Option<u16> {
        self.require_observing("wait_for_close_code");
        let deadline = Instant::now() + FRAME_TIMEOUT;
        while !self.closed && Instant::now() < deadline {
            self.drain();
        }
        self.close_code
    }

    pub fn stop_reading(&mut self) {
        self.reading = false;
    }

    fn require_observing(&self, what: &str) {
        assert!(
            self.reading,
            "{what} on a client that stopped reading — the socket is not being \
             observed, so any answer is vacuous. `stop_reading()` exists to back \
             the server's outbound queue up (WS-032); do not assert on frames \
             after it."
        );
    }

    pub fn send_json(&mut self, text: &str) {
        self.socket
            .send(Message::Text(text.into()))
            .expect("ws send");
    }

    pub fn subscribe(&mut self, channels: &str) {
        self.send_json(&subscription_request("subscribe", channels));
    }

    pub fn subscribe_logs_at(&mut self, level: &str) {
        self.send_json(
            &serde_json::json!({
                "op": "subscribe",
                "channels": ["logs"],
                "logs": { "min_level": level },
            })
            .to_string(),
        );
    }

    pub fn unsubscribe(&mut self, channels: &str) {
        self.send_json(&subscription_request("unsubscribe", channels));
    }

    pub fn close(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
    }

    pub fn ping_answered(&mut self, payload: &[u8]) -> bool {
        self.require_observing("ping_answered");
        let before = self.pongs;
        self.socket
            .send(Message::Ping(payload.to_vec().into()))
            .expect("ws ping");
        let _ = self.socket.flush();
        let deadline = Instant::now() + FRAME_TIMEOUT;
        while Instant::now() < deadline && !self.closed {
            self.drain();
            if self.pongs > before {
                return true;
            }
        }
        false
    }

    fn drain(&mut self) {
        if !self.reading || self.closed {
            return;
        }
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&text) {
                        self.pending.push(value);
                    }
                }
                Ok(Message::Pong(_)) => self.pongs += 1,
                Ok(Message::Close(frame)) => {
                    self.close_code = frame.map(|f| u16::from(f.code));
                    self.closed = true;
                    return;
                }
                Ok(_) => {}
                Err(err) => {
                    if !is_idle_timeout(&err) {
                        self.closed = true;
                    }
                    return;
                }
            }
        }
    }

    pub fn wait_for(&mut self, predicate: impl Fn(&Value) -> bool) -> Option<Value> {
        self.require_observing("wait_for");
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if let Some(pos) = self.pending.iter().position(&predicate) {
                return Some(self.pending.remove(pos));
            }
            if self.closed || Instant::now() >= deadline {
                return None;
            }
            self.drain();
        }
    }

    pub fn expect_silence(&mut self, window: Duration) -> bool {
        self.require_observing("expect_silence");
        let before = self.pending.len();
        let deadline = Instant::now() + window;
        while Instant::now() < deadline {
            self.drain();
            if self.pending.len() > before {
                return false;
            }
            if self.closed {
                return true;
            }
        }
        true
    }

    pub fn collect_for(&mut self, window: Duration) -> Vec<Value> {
        self.require_observing("collect_for");
        let from = self.pending.len();
        let deadline = Instant::now() + window;
        while Instant::now() < deadline && !self.closed {
            self.drain();
        }
        self.pending.split_off(from.min(self.pending.len()))
    }

    pub fn wait_for_op(&mut self, op: &str) -> Option<Value> {
        let op = op.to_string();
        self.wait_for(move |v| v.get("op").and_then(Value::as_str) == Some(op.as_str()))
    }

    pub fn wait_for_event(&mut self, event_type: &str, channel: &str) -> Option<Value> {
        let event_type = event_type.to_string();
        let channel = channel.to_string();
        self.wait_for(move |v| {
            v.get("type").and_then(Value::as_str) == Some(event_type.as_str())
                && v.get("channel").and_then(Value::as_str) == Some(channel.as_str())
        })
    }

    pub fn first_frame(&mut self) -> Option<Value> {
        self.require_observing("first_frame");
        let deadline = Instant::now() + FRAME_TIMEOUT;
        while self.pending.is_empty() && !self.closed && Instant::now() < deadline {
            self.drain();
        }
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.remove(0))
        }
    }
}

fn is_idle_timeout(err: &tungstenite::Error) -> bool {
    matches!(
        err,
        tungstenite::Error::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::Interrupted
            )
    )
}

fn subscription_request(op: &str, channels: &str) -> String {
    let list: Vec<&str> = channels.split(',').map(str::trim).collect();
    serde_json::json!({ "op": op, "channels": list }).to_string()
}
