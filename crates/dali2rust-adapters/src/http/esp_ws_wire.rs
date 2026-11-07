use std::sync::atomic::{AtomicBool, Ordering};

use dali2rust_ws_runtime::{WireError, WsWire};
use esp_idf_svc::sys::{
    httpd_req_async_handler_complete, httpd_req_t, lwip_recv, lwip_send, lwip_setsockopt,
    lwip_shutdown, EAGAIN, ESP_OK, EWOULDBLOCK, IPPROTO_TCP, SHUT_RD, SHUT_RDWR, SOL_SOCKET,
    SO_KEEPALIVE, TCP_KEEPCNT, TCP_KEEPIDLE, TCP_KEEPINTVL,
};

const KEEPALIVE_IDLE_S: i32 = 30;

const KEEPALIVE_INTERVAL_S: i32 = 10;

const KEEPALIVE_PROBES: i32 = 3;

pub struct EspWire {
    fd: i32,
    request: *mut httpd_req_t,
    finished: AtomicBool,
}

// SAFETY: `for_async_req` keeps the httpd task off this fd until `finish`, so nothing else holds it.
unsafe impl Send for EspWire {}
// SAFETY: held only inside `WsConnection`, which serialises writes and admits one reader.
unsafe impl Sync for EspWire {}

impl EspWire {
    fn write_all(&self, buf: &[u8]) -> Result<(), WireError> {
        let mut sent = 0;
        while sent < buf.len() {
            let rest = &buf[sent..];
            // SAFETY: `rest` is valid for `rest.len()` bytes and the fd is this connection's own.
            let wrote = unsafe { lwip_send(self.fd, rest.as_ptr().cast(), rest.len(), 0) };
            if wrote <= 0 {
                return Err(last_error());
            }
            sent += wrote as usize;
        }
        Ok(())
    }

    pub fn claim(fd: i32, request: *mut httpd_req_t) -> Self {
        arm_keepalive(fd);
        Self {
            fd,
            request,
            finished: AtomicBool::new(false),
        }
    }
}

fn set_option(fd: i32, level: u32, name: u32, value: i32) -> bool {
    let bytes = core::mem::size_of::<i32>() as u32;
    // SAFETY: `value` lives across the call and `bytes` is its own size.
    let set = unsafe {
        lwip_setsockopt(
            fd,
            level as i32,
            name as i32,
            core::ptr::addr_of!(value).cast(),
            bytes,
        )
    };
    set == ESP_OK
}

fn arm_keepalive(fd: i32) {
    let armed = set_option(fd, SOL_SOCKET, SO_KEEPALIVE, 1)
        && set_option(fd, IPPROTO_TCP, TCP_KEEPIDLE, KEEPALIVE_IDLE_S)
        && set_option(fd, IPPROTO_TCP, TCP_KEEPINTVL, KEEPALIVE_INTERVAL_S)
        && set_option(fd, IPPROTO_TCP, TCP_KEEPCNT, KEEPALIVE_PROBES);
    if !armed {
        log::warn!("ws: fd {fd} took no keepalive; a vanished peer would hold it until reboot");
    }
}

fn last_error() -> WireError {
    // SAFETY: `__errno` returns the calling task's own errno slot.
    let errno = unsafe { *esp_idf_svc::sys::__errno() };
    if errno == EAGAIN as i32 || errno == EWOULDBLOCK as i32 {
        WireError::Timeout
    } else {
        WireError::Closed
    }
}

impl WsWire for EspWire {
    fn read(&self, buf: &mut [u8]) -> Result<usize, WireError> {
        // SAFETY: `buf` is valid for `buf.len()` bytes and the fd is this connection's own.
        let got = unsafe { lwip_recv(self.fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        match got {
            0.. => Ok(got as usize),
            _ => Err(last_error()),
        }
    }

    fn write_frame(&self, header: &[u8], payload: &[u8]) -> Result<(), WireError> {
        self.write_all(header)?;
        if payload.is_empty() {
            return Ok(());
        }
        self.write_all(payload)
    }

    fn wake_reader(&self) {
        // SAFETY: the fd is this connection's own; a half shutdown only breaks a blocked read.
        unsafe { lwip_shutdown(self.fd, SHUT_RD as i32) };
    }

    fn finish(&self) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        // SAFETY: the fd is this connection's own and no owner is left to read or write it.
        unsafe { lwip_shutdown(self.fd, SHUT_RDWR as i32) };
        // SAFETY: the copy from `begin`, completed once; the server outlives the boot, so its session is live.
        let done = unsafe { httpd_req_async_handler_complete(self.request) };
        if done != ESP_OK {
            log::warn!("ws: fd {} did not complete ({done}); its slot waits on httpd", self.fd);
        }
    }
}
