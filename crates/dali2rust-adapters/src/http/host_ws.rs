use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dali2rust_ws_runtime::{ClientId, RegisterRejected, WsHub, WsSink, WsSinkError};
use tungstenite::handshake::derive_accept_key;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Message, Role, WebSocket};

pub const WS_PATH: &str = "/api/v1/ws";

const MAX_HEAD_BYTES: usize = 16 * 1024;

const ACCEPT_POLL: Duration = Duration::from_millis(50);

const HEAD_READ_TIMEOUT: Duration = Duration::from_secs(10);

const HEAD_CHUNK_BYTES: usize = 1024;

const HEAD_TERMINATOR: &[u8] = b"\r\n\r\n";

struct RequestHead {
    text: String,
    raw: Vec<u8>,
    head_len: usize,
}

impl RequestHead {
    fn path(&self) -> &str {
        self.text
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or_default()
            .split('?')
            .next()
            .unwrap_or_default()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.text.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim())
        })
    }

    fn is_ws_upgrade(&self) -> bool {
        self.path() == WS_PATH
            && self
                .header("Upgrade")
                .is_some_and(|v| v.to_ascii_lowercase().contains("websocket"))
    }

    fn origin_allowed(&self) -> bool {
        dali2rust_ws_runtime::origin_allowed(
            self.header("Origin"),
            self.header("Host"),
        )
    }

    fn remainder(&self) -> &[u8] {
        self.raw.get(self.head_len..).unwrap_or_default()
    }
}

fn head_end(raw: &[u8], from: usize) -> Option<usize> {
    raw.get(from..)?
        .windows(HEAD_TERMINATOR.len())
        .position(|window| window == HEAD_TERMINATOR)
        .map(|at| from + at + HEAD_TERMINATOR.len())
}

fn read_head(stream: &mut TcpStream) -> Option<RequestHead> {
    stream.set_read_timeout(Some(HEAD_READ_TIMEOUT)).ok()?;
    let mut raw = Vec::new();
    let mut chunk = [0u8; HEAD_CHUNK_BYTES];
    let mut scanned = 0usize;
    while raw.len() < MAX_HEAD_BYTES {
        let room = (MAX_HEAD_BYTES - raw.len()).min(chunk.len());
        match stream.read(&mut chunk[..room]) {
            Ok(0) | Err(_) => return None,
            Ok(n) => raw.extend_from_slice(&chunk[..n]),
        }
        if let Some(head_len) = head_end(&raw, scanned) {
            let text = String::from_utf8_lossy(&raw[..head_len]).to_string();
            return Some(RequestHead {
                text,
                raw,
                head_len,
            });
        }
        scanned = raw.len().saturating_sub(HEAD_TERMINATOR.len() - 1);
    }
    None
}

type WriteHalf = Arc<std::sync::Mutex<WebSocket<TcpStream>>>;

struct HostWsSink {
    socket: WriteHalf,
    closed: Arc<AtomicBool>,
}

impl WsSink for HostWsSink {
    fn send_text(&self, text: &str) -> Result<(), WsSinkError> {
        self.send(Message::Text(text.into()))
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.socket.lock() {
            let _ = guard.close(None);
            let _ = guard.flush();
        }
    }
}

impl HostWsSink {
    fn send(&self, message: Message) -> Result<(), WsSinkError> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(WsSinkError::Closed);
        }
        let mut guard = self.socket.lock().map_err(|_| WsSinkError::Failed)?;
        guard.send(message).map_err(|_| WsSinkError::Closed)
    }
}

struct ReadOnlyStream(TcpStream);

impl Read for ReadOnlyStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for ReadOnlyStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serve_connection(hub: &Arc<WsHub>, mut stream: TcpStream, inner_port: u16) {
    let head = read_head(&mut stream);
    let _ = stream.set_read_timeout(None);
    let Some(head) = head else {
        return;
    };
    if head.is_ws_upgrade() {
        upgrade(hub, stream, &head);
    } else {
        splice_to_inner(stream, &head.raw, inner_port);
    }
}

fn accept_handshake(stream: &mut TcpStream, head: &RequestHead) -> bool {
    let Some(key) = head.header("Sec-WebSocket-Key") else {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        return false;
    };
    let accept = derive_accept_key(key.as_bytes());
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).is_ok()
}

fn upgrade(hub: &Arc<WsHub>, mut stream: TcpStream, head: &RequestHead) {
    if !accept_handshake(&mut stream, head) {
        return;
    }
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let writer: WriteHalf = Arc::new(std::sync::Mutex::new(WebSocket::from_raw_socket(
        stream,
        Role::Server,
        None,
    )));
    let reader = WebSocket::from_partially_read(
        ReadOnlyStream(read_half),
        head.remainder().to_vec(),
        Role::Server,
        None,
    );
    let closed = Arc::new(AtomicBool::new(false));
    let sink = HostWsSink {
        socket: Arc::clone(&writer),
        closed: Arc::clone(&closed),
    };
    if !head.origin_allowed() {
        reject(&writer, RegisterRejected::OriginRejected);
        return;
    }
    match hub.register(Box::new(sink)) {
        Ok(id) => read_loop(Arc::clone(hub), id, reader, writer, closed),
        Err(reason) => reject(&writer, reason),
    }
}

fn reject(writer: &WriteHalf, reason: RegisterRejected) {
    let Ok(mut socket) = writer.lock() else {
        return;
    };
    let _ = socket.send(Message::Text(WsHub::rejection_frame(reason)));
    let _ = socket.close(Some(CloseFrame {
        code: CloseCode::from(dali2rust_ws_runtime::close_code(reason)),
        reason: "".into(),
    }));
    let _ = socket.flush();
}

fn read_loop(
    hub: Arc<WsHub>,
    id: ClientId,
    mut socket: WebSocket<ReadOnlyStream>,
    writer: WriteHalf,
    closed: Arc<AtomicBool>,
) {
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => hub.handle_client_text(id, &text),
            Ok(Message::Ping(payload)) => {
                if reply(&writer, Message::Pong(payload)).is_err() {
                    break;
                }
            }
            Ok(Message::Close(frame)) => {
                let _ = reply(&writer, Message::Close(frame));
                break;
            }
            Err(_) => break,
            Ok(_) => {}
        }
    }
    closed.store(true, Ordering::Relaxed);
    hub.unregister(id);
}

fn reply(writer: &WriteHalf, message: Message) -> Result<(), ()> {
    let mut socket = writer.lock().map_err(|_| ())?;
    socket.send(message).map_err(|_| ())?;
    socket.flush().map_err(|_| ())
}

fn splice_to_inner(client: TcpStream, head: &[u8], inner_port: u16) {
    let Ok(mut upstream) = TcpStream::connect(("127.0.0.1", inner_port)) else {
        return;
    };
    if upstream.write_all(head).is_err() {
        return;
    }
    let (Ok(client_rx), Ok(upstream_rx)) = (client.try_clone(), upstream.try_clone()) else {
        return;
    };
    let up = std::thread::spawn(move || pump(client_rx, upstream));
    pump(upstream_rx, client);
    let _ = up.join();
}

fn pump(mut from: TcpStream, mut to: TcpStream) {
    let mut buf = [0u8; 8192];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = to.shutdown(std::net::Shutdown::Write);
}

pub fn run_accept_loop(
    listener: &TcpListener,
    hub: Arc<WsHub>,
    inner_port: u16,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _addr)) => {
                stream.set_nonblocking(false)?;
                let hub = Arc::clone(&hub);
                std::thread::spawn(move || serve_connection(&hub, stream, inner_port));
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(ACCEPT_POLL); // sleep-ok: accept poll bounding shutdown latency
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
