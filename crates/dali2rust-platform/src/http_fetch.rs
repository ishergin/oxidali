use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub const MAX_FETCH_BYTES: usize = 64 * 1024;

pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    BadUrl,
    Transport(String),
    Status(u16),
    TooLarge,
    Malformed,
}

pub trait HttpFetch: Send + Sync {
    fn get(&self, base: &str, path: &str) -> Result<Vec<u8>, FetchError>;
}

#[must_use]
pub fn authority_of(base: &str) -> Option<String> {
    let rest = base.strip_prefix("http://")?;
    let authority = rest.split('/').next()?;
    if authority.is_empty() {
        return None;
    }
    Some(if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    })
}

#[must_use]
pub fn request_bytes(authority: &str, path: &str) -> Vec<u8> {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    )
    .into_bytes()
}

pub fn parse_response(raw: &[u8]) -> Result<(u16, Vec<u8>), FetchError> {
    let (code, head, body) = split_response(raw)?;
    let body = match framing_of(head) {
        Framing::Chunked => dechunk(body)?.ok_or(FetchError::Malformed)?,
        Framing::Length(n) => body.get(..n).ok_or(FetchError::Malformed)?.to_vec(),
        Framing::ToEof => body.to_vec(),
    };
    Ok((code, body))
}

fn split_response(raw: &[u8]) -> Result<(u16, &str, &[u8]), FetchError> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or(FetchError::Malformed)?;
    let head = core::str::from_utf8(&raw[..split]).map_err(|_| FetchError::Malformed)?;
    let status_line = head.lines().next().ok_or(FetchError::Malformed)?;
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or(FetchError::Malformed)?;
    Ok((code, head, &raw[split + 4..]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    Chunked,
    Length(usize),
    ToEof,
}

fn framing_of(head: &str) -> Framing {
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            return Framing::Chunked;
        }
        if name.eq_ignore_ascii_case("content-length") {
            if let Ok(n) = value.parse() {
                return Framing::Length(n);
            }
        }
    }
    Framing::ToEof
}

fn dechunk(mut body: &[u8]) -> Result<Option<Vec<u8>>, FetchError> {
    let mut out = Vec::new();
    loop {
        let Some(line_end) = body.windows(2).position(|w| w == b"\r\n") else {
            return Ok(None);
        };
        let size_line =
            core::str::from_utf8(&body[..line_end]).map_err(|_| FetchError::Malformed)?;
        let size = usize::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| FetchError::Malformed)?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(Some(out));
        }
        let Some(rest) = body.get(size + 2..) else {
            return Ok(None);
        };
        if &body[size..size + 2] != b"\r\n" {
            return Err(FetchError::Malformed);
        }
        out.extend_from_slice(&body[..size]);
        body = rest;
    }
}

fn response_complete(raw: &[u8]) -> bool {
    let Ok((_, head, body)) = split_response(raw) else {
        return false;
    };
    match framing_of(head) {
        Framing::Chunked => !matches!(dechunk(body), Ok(None)),
        Framing::Length(n) => body.len() >= n,
        Framing::ToEof => false,
    }
}

#[derive(Debug, Default)]
pub struct TcpHttpFetch;

impl HttpFetch for TcpHttpFetch {
    fn get(&self, base: &str, path: &str) -> Result<Vec<u8>, FetchError> {
        let authority = authority_of(base).ok_or(FetchError::BadUrl)?;
        let raw = self.exchange(&authority, path)?;
        let (code, body) = parse_response(&raw)?;
        if code != 200 {
            return Err(FetchError::Status(code));
        }
        Ok(body)
    }
}

impl TcpHttpFetch {
    fn exchange(&self, authority: &str, path: &str) -> Result<Vec<u8>, FetchError> {
        let mut stream = connect(authority)?;
        stream
            .write_all(&request_bytes(authority, path))
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        read_capped(&mut stream)
    }
}

fn connect(authority: &str) -> Result<TcpStream, FetchError> {
    let stream = TcpStream::connect(authority).map_err(|e| FetchError::Transport(e.to_string()))?;
    stream
        .set_read_timeout(Some(FETCH_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(FETCH_TIMEOUT)))
        .map_err(|e| FetchError::Transport(e.to_string()))?;
    Ok(stream)
}

fn read_capped(stream: &mut TcpStream) -> Result<Vec<u8>, FetchError> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        if read == 0 {
            return Ok(out);
        }
        if out.len() + read > MAX_FETCH_BYTES {
            return Err(FetchError::TooLarge);
        }
        out.extend_from_slice(&chunk[..read]);
        if response_complete(&out) {
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_origin_without_a_port_gets_the_default() {
        assert_eq!(authority_of("http://10.0.0.5"), Some("10.0.0.5:80".into()));
        assert_eq!(
            authority_of("http://10.0.0.5:8080"),
            Some("10.0.0.5:8080".into())
        );
    }

    #[test]
    fn a_path_on_the_origin_is_dropped_rather_than_joined() {
        assert_eq!(
            authority_of("http://host:8080/api"),
            Some("host:8080".into())
        );
    }

    #[test]
    fn anything_but_plain_http_is_refused() {
        assert_eq!(authority_of("https://host"), None);
        assert_eq!(authority_of("host:80"), None);
        assert_eq!(authority_of("http://"), None);
    }

    #[test]
    fn the_request_names_the_host_because_http_1_1_requires_it() {
        let req = String::from_utf8(request_bytes("h:80", "/api/v1/x")).expect("utf8");
        assert!(req.starts_with("GET /api/v1/x HTTP/1.1\r\n"));
        assert!(req.contains("Host: h:80\r\n"));
        assert!(req.contains("Connection: close\r\n"));
    }

    #[test]
    fn a_body_is_whatever_follows_the_blank_line() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: x\r\n\r\nbytes\r\nmore";
        let (code, body) = parse_response(raw).expect("parse");
        assert_eq!(code, 200);
        assert_eq!(body, b"bytes\r\nmore");
    }

    #[test]
    fn a_chunked_body_is_reassembled_and_the_sizes_are_not_in_it() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                    4\r\n[{\"n\r\n5;ext=1\r\n\":1}]\r\n0\r\n\r\n";
        let (code, body) = parse_response(raw).expect("parse");
        assert_eq!(code, 200);
        assert_eq!(body, b"[{\"n\":1}]");
        assert!(response_complete(raw));
    }

    #[test]
    fn a_chunked_body_still_arriving_is_incomplete_not_malformed() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nab";
        assert!(!response_complete(raw));
        assert_eq!(parse_response(raw), Err(FetchError::Malformed));
    }

    #[test]
    fn a_chunk_whose_data_is_not_followed_by_crlf_is_malformed() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nabXX0\r\n\r\n";
        assert_eq!(parse_response(raw), Err(FetchError::Malformed));
        assert!(response_complete(raw));
    }

    #[test]
    fn a_content_length_frames_the_body_and_ends_the_read() {
        let raw = b"HTTP/1.1 200 OK\r\ncontent-length: 3\r\n\r\nabc";
        assert_eq!(parse_response(raw).expect("parse").1, b"abc");
        assert!(response_complete(raw));
        assert!(!response_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nab"));
    }

    #[test]
    fn a_body_with_neither_header_ends_with_the_socket() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: x\r\n\r\nbytes";
        assert!(!response_complete(raw));
        assert_eq!(parse_response(raw).expect("parse").1, b"bytes");
    }

    #[test]
    fn an_empty_body_is_a_body() {
        let (code, body) = parse_response(b"HTTP/1.1 204 No Content\r\n\r\n").expect("parse");
        assert_eq!(code, 204);
        assert!(body.is_empty());
    }

    #[test]
    fn headers_that_never_end_are_malformed_rather_than_a_body() {
        assert_eq!(
            parse_response(b"HTTP/1.1 200 OK\r\nX: y"),
            Err(FetchError::Malformed)
        );
    }

    #[test]
    fn a_status_line_without_a_code_is_malformed() {
        assert_eq!(parse_response(b"garbage\r\n\r\n"), Err(FetchError::Malformed));
    }

    fn serve_once(response: &'static [u8]) -> (String, std::thread::JoinHandle<Vec<u8>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut request = [0u8; 512];
            let read = socket.read(&mut request).expect("read request");
            socket.write_all(response).expect("write response");
            drop(socket);
            request[..read].to_vec()
        });
        (base, handle)
    }

    #[test]
    fn a_real_socket_round_trip_sends_the_path_and_returns_the_body() {
        let (base, server) = serve_once(b"HTTP/1.1 200 OK\r\nContent-Type: x\r\n\r\nSLICEBYTES");
        let body = TcpHttpFetch
            .get(&base, "/api/v1/config/slices/groups_a0")
            .expect("fetch");
        assert_eq!(body, b"SLICEBYTES");
        let request = String::from_utf8(server.join().expect("server")).expect("utf8");
        assert!(
            request.starts_with("GET /api/v1/config/slices/groups_a0 HTTP/1.1\r\n"),
            "request was {request:?}"
        );
    }

    #[test]
    fn a_chunked_answer_on_a_socket_the_server_keeps_open_still_returns() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut request = [0u8; 512];
            let _ = socket.read(&mut request).expect("read request");
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n[1,\r\n2\r\n2]\r\n0\r\n\r\n")
                .expect("write");
            let mut sink = [0u8; 16];
            while socket.read(&mut sink).map(|n| n > 0).unwrap_or(false) {}
        });
        let started = std::time::Instant::now();
        let body = TcpHttpFetch.get(&base, "/api/v1/config/slices").expect("fetch");
        assert_eq!(body, b"[1,2]");
        assert!(
            started.elapsed() < FETCH_TIMEOUT / 2,
            "the read waited for a close instead of ending on the terminator"
        );
        let _ = server.join();
    }

    #[test]
    fn a_peer_that_answers_anything_but_200_is_a_status_error() {
        let (base, server) = serve_once(b"HTTP/1.1 404 Not Found\r\n\r\nnot_found");
        assert_eq!(
            TcpHttpFetch.get(&base, "/api/v1/config/slices/nope"),
            Err(FetchError::Status(404))
        );
        let _ = server.join();
    }

    #[test]
    fn a_peer_that_is_not_listening_is_a_transport_error() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr").port()
        };
        let base = format!("http://127.0.0.1:{port}");
        assert!(matches!(
            TcpHttpFetch.get(&base, "/api/v1/config/slices"),
            Err(FetchError::Transport(_))
        ));
    }
}

#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod crc_tests {
    use super::crc32;

    #[test]
    fn the_known_vector_holds() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn an_empty_input_is_zero() {
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_same_length_edit_changes_it() {
        assert_ne!(crc32(b"level=10"), crc32(b"level=20"));
    }
}
