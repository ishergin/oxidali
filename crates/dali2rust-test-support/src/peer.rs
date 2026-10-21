use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const MANIFEST_REQUEST: &str = "GET /api/v1/config/slices";
const EMPTY_MANIFEST: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]";
const NO_CONTENT: &[u8] = b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n";
const NOT_FOUND: &[u8] = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_REQUEST_HEAD: usize = 4096;

#[derive(Debug)]
pub struct FakePeerController {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
}

impl FakePeerController {
    #[must_use]
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake peer");
        let port = listener.local_addr().expect("fake peer address").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let server = {
            let (requests, stop) = (Arc::clone(&requests), Arc::clone(&stop));
            std::thread::spawn(move || serve(&listener, &requests, &stop))
        };
        Self {
            port,
            requests,
            stop,
            server: Some(server),
        }
    }

    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    #[must_use]
    pub fn received(&self, request_line: &str) -> usize {
        self.requests
            .lock()
            .expect("fake peer request log")
            .iter()
            .filter(|line| *line == request_line)
            .count()
    }
}

impl Drop for FakePeerController {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

fn serve(listener: &TcpListener, requests: &Mutex<Vec<String>>, stop: &AtomicBool) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(mut stream) = stream else {
            continue;
        };
        let line = request_line(&mut stream);
        let answer = answer_to(&line);
        requests.lock().expect("fake peer request log").push(line);
        let _ = stream.write_all(answer);
    }
}

fn request_line(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
    let mut head = Vec::new();
    let mut chunk = [0u8; 512];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < MAX_REQUEST_HEAD {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => head.extend_from_slice(&chunk[..read]),
        }
    }
    let text = String::from_utf8_lossy(&head);
    let first = text.lines().next().unwrap_or("");
    first
        .rsplit_once(' ')
        .map_or(first, |(line, _version)| line)
        .to_string()
}

fn answer_to(line: &str) -> &'static [u8] {
    if line == MANIFEST_REQUEST {
        EMPTY_MANIFEST
    } else if line.starts_with("POST ") {
        NO_CONTENT
    } else {
        NOT_FOUND
    }
}
