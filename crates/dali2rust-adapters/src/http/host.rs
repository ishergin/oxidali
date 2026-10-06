use std::io::Read;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tiny_http::{Header, Request, Response, Server, StatusCode};

use dali2rust_api::http::router::{BodyRefusal, Router, MAX_REQUEST_BODY_BYTES};
use dali2rust_ws_runtime::WsHub;

use super::host_ws;
use super::wire_method::wire_method;

pub struct HostServer {
    public: TcpListener,
    inner: Server,
    port: u16,
}

impl HostServer {
    pub fn bind(addr: impl std::net::ToSocketAddrs) -> Result<Self, std::io::Error> {
        let public = TcpListener::bind(addr)?;
        let port = public.local_addr()?.port();
        let inner = Server::http("127.0.0.1:0")
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(Self {
            public,
            inner,
            port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    fn inner_port(&self) -> u16 {
        self.inner
            .server_addr()
            .to_ip()
            .expect("loopback server address")
            .port()
    }
}

pub fn run_blocking(
    server: &HostServer,
    router: Arc<Router>,
    ws_hub: Arc<WsHub>,
    stop: &AtomicBool,
) -> Result<(), std::io::Error> {
    let inner_port = server.inner_port();
    std::thread::scope(|scope| {
        scope.spawn(|| host_ws::run_accept_loop(&server.public, ws_hub, inner_port, stop));
        serve_http(&server.inner, &router, stop)
    })
}

fn serve_http(
    server: &Server,
    router: &Router,
    stop: &AtomicBool,
) -> Result<(), std::io::Error> {
    while !stop.load(Ordering::Acquire) {
        match server.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(None) => continue,
            Err(e) => return Err(e),
            Ok(Some(req)) => handle_one_request(router, req),
        }
    }
    Ok(())
}

fn handle_one_request(router: &Router, mut req: Request) {
    let method = wire_method(req.method()).to_string();
    let url = req.url().to_string();
    let declared_len = declared_content_length(&req);
    let res = match read_body(declared_len, req.as_reader()) {
        Ok(body) => router.dispatch(&method, &url, &body),
        Err(refusal) => router.refuse_body(refusal),
    };
    let body_vec = res.body.into_bytes();
    let mut r = Response::from_data(body_vec).with_status_code(StatusCode(res.status));
    let ct = Header::from_bytes(&b"Content-Type"[..], res.content_type.as_bytes())
        .expect("static header");
    r.add_header(ct);
    for (name, value) in res.extra_headers {
        let header =
            Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static extra header");
        r.add_header(header);
    }
    let _ = req.respond(r);
}

fn declared_content_length(req: &Request) -> Option<usize> {
    req.headers()
        .iter()
        .find(|h| h.field.equiv("Content-Length"))
        .and_then(|h| h.value.as_str().parse::<usize>().ok())
}

fn read_body(
    declared_len: Option<usize>,
    mut reader: impl Read,
) -> Result<Vec<u8>, BodyRefusal> {
    if declared_len.is_some_and(|len| len > MAX_REQUEST_BODY_BYTES) {
        return Err(BodyRefusal::TooLarge);
    }

    let mut body_buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let remaining = MAX_REQUEST_BODY_BYTES.saturating_sub(body_buf.len());
                if remaining == 0 {
                    return Err(BodyRefusal::TooLarge);
                }
                let take = n.min(remaining);
                body_buf.extend_from_slice(&chunk[..take]);
                if take < n {
                    return Err(BodyRefusal::TooLarge);
                }
            }
            Err(_) => return Err(BodyRefusal::Incomplete),
        }
    }
    Ok(body_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn declared_length_over_limit_is_fail_fast() {
        let mut reader = Cursor::new(vec![0u8; 16]);
        let err = read_body(Some(MAX_REQUEST_BODY_BYTES + 1), &mut reader).unwrap_err();
        assert!(matches!(err, BodyRefusal::TooLarge));
        assert_eq!(reader.position(), 0);
    }

    #[test]
    fn declared_length_within_limit_reads_body() {
        let payload = vec![1u8, 2, 3];
        let mut reader = Cursor::new(payload.clone());
        let body = read_body(Some(3), &mut reader).expect("read");
        assert_eq!(body, payload);
    }

    #[test]
    fn streaming_body_over_limit_without_drain() {
        let mut reader = Cursor::new(vec![0u8; MAX_REQUEST_BODY_BYTES + 1]);
        let err = read_body(None, &mut reader).unwrap_err();
        assert!(matches!(err, BodyRefusal::TooLarge));
        assert!(reader.position() <= MAX_REQUEST_BODY_BYTES as u64 + 4096);
        assert!(reader.position() > MAX_REQUEST_BODY_BYTES as u64);
    }
}
