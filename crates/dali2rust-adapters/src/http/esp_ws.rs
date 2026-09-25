use std::sync::Arc;

use dali2rust_ws_runtime::{RegisterRejected, WsHub, WsSessions};
use esp_idf_svc::http::server::ws::EspHttpWsConnection;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::sys::{
    esp, esp_err_t, http_method_HTTP_GET, httpd_handle_t, httpd_register_uri_handler,
    httpd_req_get_hdr_value_len, httpd_req_get_hdr_value_str, httpd_req_t,
    httpd_sess_trigger_close, httpd_uri_t, httpd_ws_frame_t, httpd_ws_recv_frame, httpd_ws_type_t,
    httpd_ws_type_t_HTTPD_WS_TYPE_BINARY, httpd_ws_type_t_HTTPD_WS_TYPE_CLOSE,
    httpd_ws_type_t_HTTPD_WS_TYPE_CONTINUE, httpd_ws_type_t_HTTPD_WS_TYPE_PING,
    httpd_ws_type_t_HTTPD_WS_TYPE_PONG, httpd_ws_type_t_HTTPD_WS_TYPE_TEXT, EspError,
    ESP_ERR_INVALID_SIZE, ESP_OK,
};
use esp_idf_svc::ws::FrameType;

const WS_PATH: &str = "/api/v1/ws";

const MAX_CLIENT_FRAME_BYTES: usize = 512;

struct WsContext {
    hub: Arc<WsHub>,
    sessions: WsSessions,
    server: httpd_handle_t,
}

pub fn register_ws_handler(
    server: &mut EspHttpServer<'static>,
    hub: Arc<WsHub>,
) -> Result<(), EspError> {
    let handle = server.handle();
    let context = Box::new(WsContext {
        hub,
        sessions: WsSessions::new(),
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
    if method == http_method_HTTP_GET as i32 {
        let mut conn = EspHttpWsConnection::New(context.server, req);
        open_session(&context.hub, &context.sessions, &mut conn, context.server);
    } else {
        let mut conn = EspHttpWsConnection::Receiving(context.server, req, None);
        forward_client_frame(&context.hub, &context.sessions, &mut conn);
    }
    ESP_OK
}

fn open_session(
    hub: &Arc<WsHub>,
    sessions: &WsSessions,
    conn: &mut EspHttpWsConnection,
    server: httpd_handle_t,
) {
    let fd = conn.session();
    sessions.close(hub, fd);
    if let Some(reason) = refuse_reason(conn) {
        refuse(conn, server, fd, reason);
        return;
    }
    let session = match conn.create_detached_sender() {
        Ok(session) => session,
        Err(err) => {
            log::warn!("ws: detached sender for fd {fd} failed: {err:?}");
            close_socket(server, fd);
            return;
        }
    };
    let sink = super::esp_ws_send::DirectWsSink::new(server, fd, session);
    match hub.register(Box::new(sink)) {
        Ok(id) => sessions.insert(fd, id),
        Err(reason) => refuse(conn, server, fd, reason),
    }
}

fn refuse(
    conn: &mut EspHttpWsConnection,
    server: httpd_handle_t,
    fd: i32,
    reason: RegisterRejected,
) {
    log::info!("ws: refused upgrade on fd {fd}: {reason:?}");
    let payload = dali2rust_ws_runtime::close_code(reason).to_be_bytes();
    let _ = conn.send(FrameType::Close, &payload);
    close_socket(server, fd);
}

fn refuse_reason(conn: &EspHttpWsConnection) -> Option<RegisterRejected> {
    let origin = header(conn, c"Origin");
    let host = header(conn, c"Host");
    if !dali2rust_ws_runtime::origin_allowed(origin.as_deref(), host.as_deref()) {
        log::warn!("ws: refused upgrade from origin {origin:?} (host {host:?})");
        return Some(RegisterRejected::OriginRejected);
    }
    None
}

fn header(conn: &EspHttpWsConnection, name: &core::ffi::CStr) -> Option<String> {
    let raw_req = match conn {
        EspHttpWsConnection::New(_, req) => *req,
        EspHttpWsConnection::Receiving(_, req, _) => *req,
        EspHttpWsConnection::Closed(_) => return None,
    };
    // SAFETY: `raw_req` is esp-idf's request, live for the call; `name` is a NUL-terminated literal.
    let len = unsafe { httpd_req_get_hdr_value_len(raw_req, name.as_ptr()) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len + 1];
    // SAFETY: as above; `buf` holds `len + 1` bytes, the size passed and the most esp-idf writes.
    let err = unsafe {
        httpd_req_get_hdr_value_str(raw_req, name.as_ptr(), buf.as_mut_ptr().cast(), buf.len())
    };
    if err != ESP_OK {
        return None;
    }
    buf.truncate(len);
    String::from_utf8(buf).ok()
}

fn close_socket(server: httpd_handle_t, fd: i32) {
    // SAFETY: `server` is the handle esp-idf gave this handler and `fd` its socket.
    unsafe { httpd_sess_trigger_close(server, fd) };
}

enum ClientFrame {
    Text,
    Ignore,
    Ping,
    Close,
    Protocol,
}

#[allow(non_upper_case_globals, reason = "The opcode constants come from bindgen and are lower-case; matching on them by name is what keeps this table readable against `httpd_ws.c`")]
fn classify(frame_type: httpd_ws_type_t) -> ClientFrame {
    match frame_type {
        httpd_ws_type_t_HTTPD_WS_TYPE_TEXT => ClientFrame::Text,
        httpd_ws_type_t_HTTPD_WS_TYPE_BINARY
        | httpd_ws_type_t_HTTPD_WS_TYPE_CONTINUE
        | httpd_ws_type_t_HTTPD_WS_TYPE_PONG => ClientFrame::Ignore,
        httpd_ws_type_t_HTTPD_WS_TYPE_PING => ClientFrame::Ping,
        httpd_ws_type_t_HTTPD_WS_TYPE_CLOSE => ClientFrame::Close,
        _ => ClientFrame::Protocol,
    }
}

fn read_client_frame(
    raw_req: *mut httpd_req_t,
    buf: &mut [u8],
) -> Result<httpd_ws_frame_t, EspError> {
    let mut frame = httpd_ws_frame_t::default();
    // SAFETY: `raw_req` is live for the call and `frame` outlives it; `len` 0 selects the header parse.
    esp!(unsafe { httpd_ws_recv_frame(raw_req, &mut frame, 0) })?;
    if frame.len == 0 {
        return Ok(frame);
    }
    if frame.len > buf.len() {
        return Err(EspError::from_infallible::<ESP_ERR_INVALID_SIZE>());
    }
    frame.payload = buf.as_mut_ptr();
    // SAFETY: as above; `payload` points at `buf`, which holds at least `frame.len` bytes.
    esp!(unsafe { httpd_ws_recv_frame(raw_req, &mut frame, frame.len) })?;
    Ok(frame)
}

fn forward_client_frame(hub: &Arc<WsHub>, sessions: &WsSessions, conn: &mut EspHttpWsConnection) {
    let EspHttpWsConnection::Receiving(server, raw_req, _) = conn else {
        return;
    };
    let (server, raw_req) = (*server, *raw_req);
    let mut buf = [0u8; MAX_CLIENT_FRAME_BYTES];
    let Ok(frame) = read_client_frame(raw_req, &mut buf) else {
        fail_connection(hub, sessions, conn, server);
        return;
    };
    let payload = &buf[..frame.len];
    match classify(frame.type_) {
        ClientFrame::Text => {
            let Some(id) = sessions.lookup(conn.session()) else {
                return;
            };
            if let Ok(text) = core::str::from_utf8(payload) {
                hub.handle_client_text(id, text);
            }
        }
        ClientFrame::Ignore => {}
        ClientFrame::Ping => {
            let _ = conn.send(FrameType::Pong, payload);
        }
        ClientFrame::Close => {
            let _ = conn.send(FrameType::Close, &[]);
            let fd = conn.session();
            sessions.close(hub, fd);
            close_socket(server, fd);
        }
        ClientFrame::Protocol => fail_connection(hub, sessions, conn, server),
    }
}

fn fail_connection(
    hub: &Arc<WsHub>,
    sessions: &WsSessions,
    conn: &mut EspHttpWsConnection,
    server: httpd_handle_t,
) {
    let _ = conn.send(FrameType::Close, &[]);
    let fd = conn.session();
    sessions.close(hub, fd);
    close_socket(server, fd);
}
