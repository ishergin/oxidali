use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use dali2rust_platform::firmware::{
    FirmwareError, FirmwareImageSource, FirmwareSink, FirmwareSlot, FirmwareUpdatePort,
};

const IO_TIMEOUT: Duration = Duration::from_secs(20);

const READ_CHUNK_BYTES: usize = 8 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct Target {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Result<Target, FirmwareError> {
    let rest = url.strip_prefix("http://").ok_or(FirmwareError::BadUrl)?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().map_err(|_| FirmwareError::BadUrl)?),
        None => (authority, 80u16),
    };
    if host.is_empty() {
        return Err(FirmwareError::BadUrl);
    }
    Ok(Target {
        host: host.to_string(),
        port,
        path: path.to_string(),
    })
}

fn read_response_head(reader: &mut BufReader<TcpStream>) -> Result<Option<u32>, FirmwareError> {
    let mut status = String::new();
    reader.read_line(&mut status).map_err(|_| FirmwareError::Fetch)?;
    let code = status.split_whitespace().nth(1).unwrap_or("");
    if !code.starts_with('2') {
        return Err(FirmwareError::Fetch);
    }
    let mut total = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(|_| FirmwareError::Fetch)? == 0 {
            return Err(FirmwareError::Fetch);
        }
        let line = line.trim_end();
        if line.is_empty() {
            return Ok(total);
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                total = value.trim().parse().ok();
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct HostImageSource;

impl HostImageSource {
    fn connect(target: &Target) -> Result<TcpStream, FirmwareError> {
        let stream = TcpStream::connect((target.host.as_str(), target.port))
            .map_err(|_| FirmwareError::Fetch)?;
        stream.set_read_timeout(Some(IO_TIMEOUT)).ok();
        stream.set_write_timeout(Some(IO_TIMEOUT)).ok();
        Ok(stream)
    }

    fn request(stream: &mut TcpStream, target: &Target) -> Result<(), FirmwareError> {
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nUser-Agent: dali2rust\r\n\r\n",
            target.path, target.host
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|_| FirmwareError::Fetch)
    }
}

impl FirmwareImageSource for HostImageSource {
    fn fetch(&self, url: &str, sink: &mut dyn FirmwareSink) -> Result<u32, FirmwareError> {
        let target = parse_http_url(url)?;
        let mut stream = Self::connect(&target)?;
        Self::request(&mut stream, &target)?;
        let mut reader = BufReader::new(stream);
        let total = read_response_head(&mut reader)?;
        sink.total(total)?;
        let mut buffer = vec![0u8; READ_CHUNK_BYTES];
        let mut written = 0u32;
        loop {
            let read = reader.read(&mut buffer).map_err(|_| FirmwareError::Fetch)?;
            if read == 0 {
                break;
            }
            sink.chunk(&buffer[..read])?;
            written = written.saturating_add(read as u32);
        }
        if total.is_some_and(|announced| announced != written) {
            return Err(FirmwareError::Fetch);
        }
        Ok(written)
    }
}

#[derive(Debug, Default)]
pub struct SimFirmwarePort {
    written: AtomicU32,
    open: AtomicBool,
    finished: AtomicBool,
    rebooted: AtomicBool,
    marked_valid: AtomicBool,
    reject_image: AtomicBool,
    last_total: Mutex<Option<u32>>,
}

impl SimFirmwarePort {
    pub fn written_bytes(&self) -> u32 {
        self.written.load(Ordering::Relaxed)
    }

    pub fn rebooted(&self) -> bool {
        self.rebooted.load(Ordering::Relaxed)
    }

    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    pub fn marked_valid(&self) -> bool {
        self.marked_valid.load(Ordering::Relaxed)
    }

    pub fn announced_total(&self) -> Option<u32> {
        *self.last_total.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn reject_next_image(&self) {
        self.reject_image.store(true, Ordering::Relaxed);
    }
}

impl FirmwareUpdatePort for SimFirmwarePort {
    fn slot(&self) -> FirmwareSlot {
        FirmwareSlot::new("host", false, true)
    }

    fn begin(&self, total_bytes: Option<u32>) -> Result<(), FirmwareError> {
        *self.last_total.lock().unwrap_or_else(|e| e.into_inner()) = total_bytes;
        self.written.store(0, Ordering::Relaxed);
        self.finished.store(false, Ordering::Relaxed);
        self.open.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn write(&self, chunk: &[u8]) -> Result<(), FirmwareError> {
        if !self.open.load(Ordering::Relaxed) {
            return Err(FirmwareError::Write);
        }
        self.written.fetch_add(chunk.len() as u32, Ordering::Relaxed);
        Ok(())
    }

    fn finish(&self) -> Result<(), FirmwareError> {
        if !self.open.swap(false, Ordering::Relaxed) {
            return Err(FirmwareError::Write);
        }
        if self.reject_image.swap(false, Ordering::Relaxed) {
            return Err(FirmwareError::InvalidImage);
        }
        self.finished.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn abort(&self) {
        self.open.store(false, Ordering::Relaxed);
    }

    fn mark_valid(&self) -> Result<(), FirmwareError> {
        self.marked_valid.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn reboot(&self) {
        self.rebooted.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_split_into_host_port_and_path() {
        let t = parse_http_url("http://192.168.11.9:8080/fw/dali2rust.bin").unwrap();
        assert_eq!((t.host.as_str(), t.port, t.path.as_str()),
                   ("192.168.11.9", 8080, "/fw/dali2rust.bin"));
        let default_port = parse_http_url("http://host/x.bin").unwrap();
        assert_eq!((default_port.port, default_port.path.as_str()), (80, "/x.bin"));
        assert_eq!(parse_http_url("http://host").unwrap().path, "/");
    }

    #[test]
    fn https_is_not_pretended_on_the_host() {
        assert_eq!(parse_http_url("https://host/x.bin"), Err(FirmwareError::BadUrl));
    }

    #[test]
    fn a_closed_slot_refuses_writes_and_finishes() {
        let port = SimFirmwarePort::default();
        assert_eq!(port.write(b"x"), Err(FirmwareError::Write));
        assert_eq!(port.finish(), Err(FirmwareError::Write));
        port.begin(Some(4)).unwrap();
        port.write(b"abcd").unwrap();
        port.finish().unwrap();
        assert_eq!(port.written_bytes(), 4);
        assert!(port.finished());
    }
}
