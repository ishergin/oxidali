use std::sync::atomic::{AtomicBool, Ordering};

use dali2rust_ws_runtime::{WireError, WsWire};
use esp_idf_svc::sys::{
    httpd_req_async_handler_complete, httpd_req_t, lwip_recv, lwip_send, lwip_shutdown, EAGAIN,
    ETIMEDOUT, SHUT_RDWR, SHUT_RD,
};

pub struct EspWire {
    fd: i32,
    request: *mut httpd_req_t,
    finished: AtomicBool,
}

// SAFETY: the request copy is this connection's own, and `for_async_req` keeps the httpd task
// off this socket until `finish`, so the fd and the copy have no other user.
unsafe impl Send for EspWire {}
// SAFETY: as above; the reader and the sender share it, and writes are serialised above.
unsafe impl Sync for EspWire {}

impl EspWire {
    pub const fn new(fd: i32, request: *mut httpd_req_t) -> Self {
        Self {
            fd,
            request,
            finished: AtomicBool::new(false),
        }
    }
}

fn last_error() -> WireError {
    // SAFETY: `__errno` returns the calling task's own errno slot.
    let errno = unsafe { *esp_idf_svc::sys::__errno() };
    if errno == EAGAIN as i32 || errno == ETIMEDOUT as i32 {
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
        // SAFETY: the request copy came from `httpd_req_async_handler_begin` and is completed
        // once; esp-idf frees it here and takes the socket back onto its own task.
        unsafe { httpd_req_async_handler_complete(self.request) };
    }
}
