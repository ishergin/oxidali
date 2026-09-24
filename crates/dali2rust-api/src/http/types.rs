use std::io::{self, Write};

const HTTP_BODY_WRITE_BUFFER_CAPACITY: usize = 2048;

struct FixedBufWriter<'a, const N: usize> {
    inner: &'a mut dyn Write,
    buf: [u8; N],
    len: usize,
}

impl<'a, const N: usize> FixedBufWriter<'a, N> {
    fn new(inner: &'a mut dyn Write) -> Self {
        Self {
            inner,
            buf: [0; N],
            len: 0,
        }
    }

    fn flush_buffer(&mut self) -> io::Result<()> {
        if self.len > 0 {
            self.inner.write_all(&self.buf[..self.len])?;
            self.len = 0;
        }
        Ok(())
    }
}

impl<const N: usize> Write for FixedBufWriter<'_, N> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        if self.len == 0 && buf.len() >= N {
            self.inner.write_all(buf)?;
            return Ok(buf.len());
        }

        let remaining = N - self.len;
        if buf.len() > remaining {
            self.flush_buffer()?;
            if buf.len() >= N {
                self.inner.write_all(buf)?;
                return Ok(buf.len());
            }
        }

        let end = self.len + buf.len();
        self.buf[self.len..end].copy_from_slice(buf);
        self.len = end;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_buffer()?;
        self.inner.flush()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HttpMethod {
    Get,
    Post,
    Patch,
    Put,
    Delete,
    Other,
}

impl HttpMethod {
    #[allow(
        clippy::should_implement_trait,
        reason = "from_request is a constructor that may fail, not a From impl"
    )]
    pub fn from_str(s: &str) -> Self {
        match s {
            "GET" | "get" => Self::Get,
            "POST" | "post" => Self::Post,
            "PATCH" | "patch" => Self::Patch,
            "PUT" | "put" => Self::Put,
            "DELETE" | "delete" => Self::Delete,
            _ => Self::Other,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Patch => "PATCH",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Other => "OTHER",
        }
    }
}

pub enum HttpBody {
    Buffered(Vec<u8>),
    Stream(Box<dyn FnOnce(&mut dyn Write) -> io::Result<()> + Send>),
}

impl HttpBody {
    pub fn write_to(self, out: &mut dyn Write) -> io::Result<()> {
        let mut out = FixedBufWriter::<HTTP_BODY_WRITE_BUFFER_CAPACITY>::new(out);
        match self {
            HttpBody::Buffered(b) => out.write_all(&b)?,
            HttpBody::Stream(f) => f(&mut out)?,
        }
        out.flush()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.write_to(&mut buf).expect("http streaming body write");
        buf
    }
}

pub const NO_EXTRA_HEADERS: &[(&str, &str)] = &[];

pub struct HttpResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub extra_headers: &'static [(&'static str, &'static str)],
    pub body: HttpBody,
}

impl HttpResponse {
    pub fn json(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: "application/json",
            extra_headers: NO_EXTRA_HEADERS,
            body: HttpBody::Buffered(body),
        }
    }

    pub fn json_stream(
        status: u16,
        write: Box<dyn FnOnce(&mut dyn Write) -> io::Result<()> + Send>,
    ) -> Self {
        Self {
            status,
            content_type: "application/json",
            extra_headers: NO_EXTRA_HEADERS,
            body: HttpBody::Stream(write),
        }
    }

    pub fn not_found() -> Self {
        Self::json(404, br#"{"error":"not_found"}"#.to_vec())
    }

    pub fn method_not_allowed() -> Self {
        Self::json(405, br#"{"error":"method_not_allowed"}"#.to_vec())
    }

    pub fn no_content() -> Self {
        Self {
            status: 204,
            content_type: "application/json",
            extra_headers: NO_EXTRA_HEADERS,
            body: HttpBody::Buffered(Vec::new()),
        }
    }

    pub fn into_body_bytes(self) -> Vec<u8> {
        self.body.into_bytes()
    }
}

#[derive(Debug, Clone)]
pub struct RouteRegisterError(pub String);

impl core::fmt::Display for RouteRegisterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn json_stream_writes_compact_json() {
        let res = HttpResponse::json_stream(
            200,
            Box::new(|w| {
                write!(w, "{{\"adapter_id\":1,\"virtual_lamps\":[")?;
                write!(w, "{{\"x\":2}}")?;
                write!(w, "]}}")?;
                Ok(())
            }),
        );
        assert_eq!(
            String::from_utf8_lossy(&res.into_body_bytes()),
            r#"{"adapter_id":1,"virtual_lamps":[{"x":2}]}"#
        );
    }

    #[test]
    fn http_body_buffered_into_bytes_moves_inner() {
        let b = br#"{"ok":true}"#.to_vec();
        let out = HttpBody::Buffered(b.clone()).into_bytes();
        assert_eq!(out, b);
    }

    #[test]
    fn http_body_stream_write_to_writes_incrementally() {
        let mut out = Vec::new();
        HttpBody::Stream(Box::new(|w| {
            w.write_all(br#"{"items":["#)?;
            w.write_all(br#"{"id":1}"#)?;
            w.write_all(br#"]}"#)?;
            Ok(())
        }))
        .write_to(&mut out)
        .expect("write body");
        assert_eq!(String::from_utf8_lossy(&out), r#"{"items":[{"id":1}]}"#);
    }

    #[derive(Default)]
    struct CountingWriter {
        bytes: Vec<u8>,
        write_calls: usize,
    }

    impl Write for CountingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.write_calls += 1;
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn http_body_stream_write_to_coalesces_small_writes() {
        let mut out = CountingWriter::default();
        HttpBody::Stream(Box::new(|w| {
            w.write_all(b"{")?;
            w.write_all(b"\"items\"")?;
            w.write_all(b":[")?;
            w.write_all(b"{\"id\":1}")?;
            w.write_all(b"]}")?;
            Ok(())
        }))
        .write_to(&mut out)
        .expect("write body");

        assert_eq!(
            String::from_utf8_lossy(&out.bytes),
            r#"{"items":[{"id":1}]}"#
        );
        assert_eq!(out.write_calls, 1);
    }
}
