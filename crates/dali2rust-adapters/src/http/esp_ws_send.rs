use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

use dali2rust_ws_runtime::{WsSink, WsSinkError};
use esp_idf_svc::http::server::ws::EspHttpWsDetachedSender;
use esp_idf_svc::sys::{
    esp_err_t, httpd_handle_t, httpd_queue_work, httpd_ws_frame_t, httpd_ws_send_frame_async,
    httpd_ws_type_t, httpd_ws_type_t_HTTPD_WS_TYPE_CLOSE, httpd_ws_type_t_HTTPD_WS_TYPE_TEXT,
    ESP_FAIL, ESP_OK,
};

struct SendSlot {
    done: Mutex<Option<esp_err_t>>,
    ready: Condvar,
}

struct SendRequest {
    server: httpd_handle_t,
    fd: i32,
    frame: httpd_ws_frame_t,
    slot: *const SendSlot,
    session: *const EspHttpWsDetachedSender,
}

pub struct DirectWsSink {
    server: httpd_handle_t,
    fd: i32,
    slot: SendSlot,
    closed: AtomicBool,
    session: EspHttpWsDetachedSender,
}

// SAFETY: `server` is usable from any task via `httpd_queue_work`; `session` is only asked `is_closed`.
unsafe impl Send for DirectWsSink {}
unsafe impl Sync for DirectWsSink {}

impl DirectWsSink {
    pub fn new(server: httpd_handle_t, fd: i32, session: EspHttpWsDetachedSender) -> Self {
        Self {
            session,
            server,
            fd,
            slot: SendSlot {
                done: Mutex::new(None),
                ready: Condvar::new(),
            },
            closed: AtomicBool::new(false),
        }
    }

    fn send_frame(&self, kind: httpd_ws_type_t, payload: &[u8]) -> Result<(), WsSinkError> {
        if self.closed.load(Ordering::Acquire) || self.session.is_closed() {
            self.closed.store(true, Ordering::Release);
            return Err(WsSinkError::Closed);
        }
        *self.slot.done.lock().map_err(|_| WsSinkError::Failed)? = None;
        let mut request = SendRequest {
            server: self.server,
            fd: self.fd,
            frame: httpd_ws_frame_t {
                final_: true,
                fragmented: false,
                type_: kind,
                payload: payload.as_ptr().cast_mut(),
                len: payload.len(),
            },
            slot: &self.slot,
            session: &self.session,
        };
        // SAFETY: `request` outlives the work item: this thread blocks until the item signals under the slot lock.
        let queued = unsafe {
            httpd_queue_work(self.server, Some(send_on_httpd_task), (&raw mut request).cast())
        };
        if queued != ESP_OK {
            self.closed.store(true, Ordering::Release);
            return Err(WsSinkError::Closed);
        }
        self.wait_for_outcome()
    }

    fn wait_for_outcome(&self) -> Result<(), WsSinkError> {
        let mut done = self.slot.done.lock().map_err(|_| WsSinkError::Failed)?;
        while done.is_none() {
            done = self.slot.ready.wait(done).map_err(|_| WsSinkError::Failed)?;
        }
        if *done == Some(ESP_OK) {
            Ok(())
        } else {
            self.closed.store(true, Ordering::Release);
            Err(WsSinkError::Closed)
        }
    }
}

unsafe extern "C" fn send_on_httpd_task(arg: *mut core::ffi::c_void) {
    // SAFETY: `arg` is the `SendRequest` queued by `send_frame`, alive until the slot says done.
    let request = unsafe { &mut *arg.cast::<SendRequest>() };
    // SAFETY: the sink owning `session` is blocked on this request's slot.
    let result = if unsafe { &*request.session }.is_closed() {
        ESP_FAIL
    } else {
        // SAFETY: the handle and socket the request names; the payload lives as long as the request.
        unsafe { httpd_ws_send_frame_async(request.server, request.fd, &mut request.frame) }
    };
    // SAFETY: the slot belongs to the sink whose thread is blocked on it.
    let slot = unsafe { &*request.slot };
    if let Ok(mut done) = slot.done.lock() {
        *done = Some(result);
        slot.ready.notify_one();
    }
}

impl WsSink for DirectWsSink {
    fn send_text(&self, text: &str) -> Result<(), WsSinkError> {
        self.send_frame(httpd_ws_type_t_HTTPD_WS_TYPE_TEXT, text.as_bytes())
    }

    fn close(&self) {
        let _ = self.send_frame(httpd_ws_type_t_HTTPD_WS_TYPE_CLOSE, &[]);
    }
}
