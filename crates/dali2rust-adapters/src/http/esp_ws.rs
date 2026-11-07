use std::sync::Arc;

use dali2rust_ws_runtime::{
    close_frame, close_code, serve_client_frames, ClientId, Opcode, RegisterRejected, WsConnection,
    WsHub, WsSink, WsSinkError, MAX_CLIENT_FRAME_BYTES,
};
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::sys::{
    esp, esp_err_t, http_method_HTTP_GET, httpd_handle_t, httpd_register_uri_handler,
    httpd_req_async_handler_begin, httpd_req_get_hdr_value_len, httpd_req_get_hdr_value_str,
    httpd_req_t, httpd_req_to_sockfd, httpd_sess_trigger_close, httpd_uri_t, lwip_send, EspError,
    ESP_OK,
};

use super::esp_ws_wire::EspWire;

const WS_PATH: &str = "/api/v1/ws";

const MAX_CLOSE_FRAME_BYTES: usize = 8;

type Connection = Arc<WsConnection<EspWire>>;

struct WsContext {
    hub: Arc<WsHub>,
    server: httpd_handle_t,
}

struct ConnSink(Connection);

impl WsSink for ConnSink {
    fn send_batch(&self, frames: &[Arc<str>]) -> Result<(), WsSinkError> {
        for frame in frames {
            self.0
                .send_frame(Opcode::Text, frame.as_bytes())
                .map_err(|_| WsSinkError::Closed)?;
        }
        Ok(())
    }

    fn close(&self) {
        let _ = self.0.send_frame(Opcode::Close, &[]);
        self.0.wake_reader();
    }
}

pub fn register_ws_handler(
    server: &mut EspHttpServer<'static>,
    hub: Arc<WsHub>,
) -> Result<(), EspError> {
    let handle = server.handle();
    let context = Box::new(WsContext {
        hub,
        server: handle,
    });
    // SAFETY: one leak at boot; esp-idf keeps `user_ctx` for the registration, which is never removed.
    let context: &'static WsContext = Box::leak(context);
    let uri = c"/api/v1/ws";
    debug_assert_eq!(uri.to_bytes(), WS_PATH.as_bytes());
    let config = httpd_uri_t {
        uri: uri.as_ptr(),
        method: http_method_HTTP_GET,
        handler: Some(ws_uri_handler),
        user_ctx: (context as *const WsContext as *mut WsContext).cast(),
        is_websocket: true,
        handle_ws_control_frames: true,
        ..Default::default()
    };
    // SAFETY: `config` lives across the call; the URI is a `'static` C literal and the context is leaked.
    esp!(unsafe { httpd_register_uri_handler(handle, &config) })?;
    Ok(())
}

extern "C" fn ws_uri_handler(req: *mut httpd_req_t) -> esp_err_t {
    // SAFETY: esp-idf passes a request it owns for the call, carrying the `user_ctx` registered above.
    let Some(context) = (unsafe { (*req).user_ctx.cast::<WsContext>().as_ref() }) else {
        return ESP_OK;
    };
    // SAFETY: as above.
    let method = unsafe { (*req).method };
    // SAFETY: as above; the request is live for the call.
    let fd = unsafe { httpd_req_to_sockfd(req) };
    if method == http_method_HTTP_GET as i32 {
        open_session(context, req, fd);
    } else {
        close_socket(context.server, fd);
    }
    ESP_OK
}

fn open_session(context: &'static WsContext, req: *mut httpd_req_t, fd: i32) {
    if let Some(reason) = refuse_reason(req) {
        refuse_before_handover(context.server, fd, reason);
        return;
    }
    let Some(conn) = hand_over(req, fd) else {
        close_socket(context.server, fd);
        return;
    };
    match context.hub.register(Box::new(ConnSink(Arc::clone(&conn)))) {
        Ok(id) => start_reader(&context.hub, id, conn),
        Err(reason) => refuse_after_handover(&conn, fd, reason),
    }
}

fn hand_over(req: *mut httpd_req_t, fd: i32) -> Option<Connection> {
    let mut handed: *mut httpd_req_t = core::ptr::null_mut();
    // SAFETY: the request is live for the call; esp-idf allocates the copy and hands it back.
    let begun = esp!(unsafe { httpd_req_async_handler_begin(req, &mut handed) });
    if begun.is_err() || handed.is_null() {
        log::warn!("ws: fd {fd} could not be handed to a worker");
        return None;
    }
    Some(Arc::new(WsConnection::new(EspWire::new(fd, handed))))
}

fn start_reader(hub: &Arc<WsHub>, id: ClientId, conn: Connection) {
    let reader_hub = Arc::clone(hub);
    let started = hub.start_reader(move || {
        let mut buffer = vec![0u8; MAX_CLIENT_FRAME_BYTES];
        let exit = serve_client_frames(reader_hub.as_ref(), id, conn.as_ref(), &mut buffer);
        log::info!("ws: client {id} reader ended ({exit:?})");
    });
    if let Err(err) = started {
        log::warn!("ws: no reader for client {id} ({err}); dropping it");
        hub.unregister(id);
    }
}

fn refuse_after_handover(conn: &Connection, fd: i32, reason: RegisterRejected) {
    log::info!("ws: refused upgrade on fd {fd}: {reason:?}");
    let payload = close_code(reason).to_be_bytes();
    let _ = conn.send_frame(Opcode::Close, &payload);
}

fn refuse_before_handover(server: httpd_handle_t, fd: i32, reason: RegisterRejected) {
    log::info!("ws: refused upgrade on fd {fd}: {reason:?}");
    let payload = close_code(reason).to_be_bytes();
    let mut frame = [0u8; MAX_CLOSE_FRAME_BYTES];
    if let Some(len) = close_frame(&payload, &mut frame) {
        // SAFETY: on the httpd task inside this handler the fd is live; `frame` lives for the call.
        unsafe { lwip_send(fd, frame.as_ptr().cast(), len, 0) };
    }
    close_socket(server, fd);
}

fn refuse_reason(req: *mut httpd_req_t) -> Option<RegisterRejected> {
    let origin = header(req, c"Origin");
    let host = header(req, c"Host");
    if !dali2rust_ws_runtime::origin_allowed(origin.as_deref(), host.as_deref()) {
        log::warn!("ws: refused upgrade from origin {origin:?} (host {host:?})");
        return Some(RegisterRejected::OriginRejected);
    }
    None
}

fn header(req: *mut httpd_req_t, name: &core::ffi::CStr) -> Option<String> {
    // SAFETY: the request is live for the call; `name` is a NUL-terminated literal.
    let len = unsafe { httpd_req_get_hdr_value_len(req, name.as_ptr()) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len + 1];
    // SAFETY: as above; `buf` holds `len + 1` bytes, the size passed and the most esp-idf writes.
    let err = unsafe {
        httpd_req_get_hdr_value_str(req, name.as_ptr(), buf.as_mut_ptr().cast(), buf.len())
    };
    if err != ESP_OK {
        return None;
    }
    buf.truncate(len);
    String::from_utf8(buf).ok()
}

fn close_socket(server: httpd_handle_t, fd: i32) {
    // SAFETY: on the httpd task `server` is the handle this handler was given and `fd` its socket.
    unsafe { httpd_sess_trigger_close(server, fd) };
}
