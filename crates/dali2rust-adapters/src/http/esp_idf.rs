use std::io;
use std::io::Write as StdWrite;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use esp_idf_svc::http::server::{Configuration, EspHttpConnection, EspHttpServer, Method, Request};
use esp_idf_svc::io::{EspIOError, Write};
use esp_idf_svc::sys::{EspError, ESP_FAIL};

use dali2rust_api::http::router::{BodyRefusal, Router, MAX_REQUEST_BODY_BYTES};
use dali2rust_api::http::types::{HttpBody, HttpResponse};

use super::wire_method::wire_method;

const HTTPD_TASK_STACK_BYTES: usize = 20 * 1024;
const HTTPD_MAX_OPEN_SOCKETS: usize = 10;

pub struct HttpdStackReserve(*mut core::ffi::c_void);

// SAFETY: the pointer is an owned, never-aliased heap allocation, only ever moved between threads.
unsafe impl Send for HttpdStackReserve {}

impl HttpdStackReserve {
    const RESERVE_BYTES: usize = HTTPD_TASK_STACK_BYTES + 12 * 1024;

    pub fn take() -> Self {
        use esp_idf_svc::sys::{heap_caps_malloc, MALLOC_CAP_8BIT, MALLOC_CAP_INTERNAL};
        if httpd_stack_in_psram() {
            return Self(core::ptr::null_mut());
        }
        // SAFETY: plain capability-tagged allocation; freed in `Drop`.
        let ptr = unsafe {
            heap_caps_malloc(Self::RESERVE_BYTES, MALLOC_CAP_INTERNAL | MALLOC_CAP_8BIT)
        };
        if ptr.is_null() {
            log::error!(
                "HTTP: httpd stack reserve failed ({} B)",
                Self::RESERVE_BYTES
            );
        }
        Self(ptr)
    }
}

fn httpd_stack_in_psram() -> bool {
    dali2rust_bsp::esp_thread::external_stacks_on_xip()
}

impl Drop for HttpdStackReserve {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: pointer originates from `heap_caps_malloc` above.
            unsafe { esp_idf_svc::sys::heap_caps_free(self.0) };
        }
    }
}

fn log_internal_heap_before_httpd() {
    #[cfg(target_os = "espidf")]
    {
        let h = dali2rust_bsp::heap_stats::EspHeapStats.boot_snapshot();
        log::warn!(
            "HTTP: internal heap before httpd: free={} B, largest_block={} B",
            h.internal_free_bytes,
            h.internal_largest_block_bytes
        );
    }
}

fn httpd_configuration() -> Configuration {
    let defaults = Configuration::default();
    let task_caps = if httpd_stack_in_psram() {
        esp_idf_svc::sys::MALLOC_CAP_SPIRAM | esp_idf_svc::sys::MALLOC_CAP_8BIT
    } else {
        defaults.task_caps
    };
    Configuration {
        stack_size: HTTPD_TASK_STACK_BYTES,
        uri_match_wildcard: true,
        max_open_sockets: HTTPD_MAX_OPEN_SOCKETS,
        task_caps,
        ..defaults
    }
}

#[inline(never)]
pub fn mount(
    router: Arc<Router>,
    ws_hub: Arc<dali2rust_ws_runtime::WsHub>,
    stack_reserve: HttpdStackReserve,
) -> Result<EspHttpServer<'static>, EspIOError> {
    drop(stack_reserve);
    log_internal_heap_before_httpd();
    let conf = httpd_configuration();

    assert_eq!(
        conf.stack_size, HTTPD_TASK_STACK_BYTES,
        "httpd stack_size must be set; check httpd_configuration() and profile optimizations"
    );

    log::warn!(
        "HTTP: httpd stack_size={} bytes in {}, max_open_sockets={}, uri_match_wildcard={}",
        conf.stack_size,
        if conf.task_caps & esp_idf_svc::sys::MALLOC_CAP_SPIRAM != 0 { "PSRAM" } else { "internal SRAM" },
        conf.max_open_sockets,
        conf.uri_match_wildcard
    );

    let mut server = EspHttpServer::new(&conf)?;

    super::esp_ws::register_ws_handler(&mut server, ws_hub)?;

    const METHODS: &[Method] = &[
        Method::Get,
        Method::Post,
        Method::Put,
        Method::Delete,
        Method::Patch,
    ];

    for &method in METHODS {
        let r = router.clone();
        server.fn_handler("/*", method, move |req| handle_request(&r, req))?;
    }

    Ok(server)
}

const ACCESS_BODY_PREVIEW_BYTES: usize = 256;

const RESPONSE_BUFFER_BYTES: usize = 2048;

fn heap_free_and_largest_at_boot() -> (usize, usize) {
    use dali2rust_platform::heap::HeapStatsPort;

    let h = dali2rust_bsp::heap_stats::EspHeapStats.snapshot();
    (
        h.internal_free_bytes as usize,
        h.internal_largest_block_bytes as usize,
    )
}

static HTTPD_CORE_ID: AtomicU32 = AtomicU32::new(u32::MAX);

pub fn httpd_core_id() -> Option<u32> {
    match HTTPD_CORE_ID.load(Ordering::Relaxed) {
        u32::MAX => None,
        id => Some(id),
    }
}

struct StdIoBridge<'a, W>(&'a mut W);

impl<W> StdWrite for StdIoBridge<'_, W>
where
    W: Write<Error = EspIOError>,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Write::write(self.0, buf)
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "esp http write failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Write::flush(self.0)
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "esp http flush failed"))
    }
}

#[derive(Default)]
struct SocketMeter {
    micros: u64,
    bytes: usize,
    calls: u32,
}

impl SocketMeter {
    fn millis(&self) -> u64 {
        self.micros / 1000
    }
}

struct MeteredWrite<'m, W> {
    inner: W,
    meter: &'m mut SocketMeter,
}

impl<W: StdWrite> StdWrite for MeteredWrite<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let started = std::time::Instant::now();
        let result = self.inner.write(buf);
        self.meter.micros += started.elapsed().as_micros() as u64;
        self.meter.calls += 1;
        if let Ok(n) = result {
            self.meter.bytes += n;
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        let started = std::time::Instant::now();
        let result = self.inner.flush();
        self.meter.micros += started.elapsed().as_micros() as u64;
        result
    }
}

fn read_body(req: &mut Request<&mut EspHttpConnection<'_>>) -> Result<Vec<u8>, BodyRefusal> {
    let len = req
        .header("Content-Length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if len > MAX_REQUEST_BODY_BYTES {
        return Err(BodyRefusal::TooLarge);
    }
    let mut buf = vec![0u8; len];
    if len == 0 {
        return Ok(buf);
    }
    match esp_idf_svc::io::utils::try_read_full(req, &mut buf) {
        Ok(_) => Ok(buf),
        Err(_) => Err(BodyRefusal::Incomplete),
    }
}

fn body_preview(body: &[u8]) -> String {
    let take = body.len().min(ACCESS_BODY_PREVIEW_BYTES);
    let preview = String::from_utf8_lossy(&body[..take]);
    let suffix = if body.len() > take { "..." } else { "" };
    format!("{preview:?}{suffix} ({} bytes)", body.len())
}

fn log_access(method: &str, uri: &str, body: &[u8], status: u16) {
    if body.is_empty() {
        log::info!("HTTP access: {method} {uri} -> {status}");
        return;
    }
    log::info!(
        "HTTP access: {method} {uri} -> {status}; body={}",
        body_preview(body)
    );
}

fn log_access_error(method: &str, uri: &str, status: u16, detail: &str) {
    log::warn!("HTTP access: {method} {uri} -> {status}; {detail}");
}

fn response_headers(res: &HttpResponse) -> Vec<(&'static str, &'static str)> {
    let mut headers = Vec::with_capacity(1 + res.extra_headers.len());
    headers.push(("Content-Type", res.content_type));
    headers.extend_from_slice(res.extra_headers);
    headers
}

fn note_httpd_stack_low_water() {
    use core::sync::atomic::{AtomicU32, Ordering};
    static MIN_FREE: AtomicU32 = AtomicU32::new(u32::MAX);
    // SAFETY: pure FFI query about the current task; no aliasing, no state.
    let free = unsafe {
        esp_idf_svc::sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut())
    };
    let prev = MIN_FREE.load(Ordering::Relaxed);
    if free < prev {
        MIN_FREE.store(free, Ordering::Relaxed);
        log::warn!("httpd stack low-water: {free} B free of {HTTPD_TASK_STACK_BYTES}");
    }
    dali2rust_bsp::task_registry::observe_current_with_free(c"httpd", free as u32);
}

const SLOW_REQUEST_MS: u64 = 2_000;

const TCP_MSS_BYTES: usize = 1_440;

const SLOW_SOCKET_MS: u64 = 1_000;

#[derive(Clone, Copy, Default)]
struct LwipCounters {
    tcp_xmit: u16,
    tcp_drop: u16,
    link_drop: u16,
    link_memerr: u16,
}

impl LwipCounters {
    fn snapshot() -> Self {
        // SAFETY: raw-pointer read of lwIP's global counters, never a reference; a torn read skews one log line.
        unsafe {
            let s = core::ptr::addr_of!(esp_idf_svc::sys::lwip_stats);
            Self {
                tcp_xmit: (*s).tcp.xmit,
                tcp_drop: (*s).tcp.drop,
                link_drop: (*s).link.drop,
                link_memerr: (*s).link.memerr,
            }
        }
    }

    fn since(&self, start: Self) -> Self {
        Self {
            tcp_xmit: self.tcp_xmit.wrapping_sub(start.tcp_xmit),
            tcp_drop: self.tcp_drop.wrapping_sub(start.tcp_drop),
            link_drop: self.link_drop.wrapping_sub(start.link_drop),
            link_memerr: self.link_memerr.wrapping_sub(start.link_memerr),
        }
    }
}

struct Phases {
    dispatch_ms: u64,
    log_ms: u64,
    hdr_ms: u64,
    body_ms: u64,
    total_ms: u64,
}

fn handle_request(
    router: &Arc<Router>,
    mut req: Request<&mut EspHttpConnection<'_>>,
) -> Result<(), EspIOError> {
    let method = wire_method(req.method());
    let uri = req.uri().to_string();
    match read_body(&mut req) {
        Ok(body) => serve(router, req, method, &uri, &body),
        Err(refusal) => respond_read_error(router, req, method, &uri, refusal),
    }
}

fn respond_read_error(
    router: &Router,
    req: Request<&mut EspHttpConnection<'_>>,
    method: &str,
    uri: &str,
    refusal: BodyRefusal,
) -> Result<(), EspIOError> {
    log_access_error(method, uri, refusal.status(), refusal.code());
    let res = router.refuse_body(refusal);
    let mut w = req.into_response(res.status, None, &response_headers(&res))?;
    w.write_all(&res.body.into_bytes())?;
    Ok(())
}

fn serve(
    router: &Arc<Router>,
    req: Request<&mut EspHttpConnection<'_>>,
    method: &str,
    uri: &str,
    body: &[u8],
) -> Result<(), EspIOError> {
    HTTPD_CORE_ID.store(esp_idf_svc::hal::cpu::core() as u32, Ordering::Relaxed);
    let started = std::time::Instant::now();
    let res = router.dispatch(method, uri, body);
    note_httpd_stack_low_water();
    let dispatch_ms = started.elapsed().as_millis() as u64;

    let status = res.status;
    log_access(method, uri, body, status);
    let log_ms = (started.elapsed().as_millis() as u64).saturating_sub(dispatch_ms);

    let headers = response_headers(&res);
    let mut w = req.into_response(status, None, &headers)?;
    let hdr_ms = (started.elapsed().as_millis() as u64).saturating_sub(dispatch_ms + log_ms);

    let mut meter = SocketMeter::default();
    let lwip_before = LwipCounters::snapshot();
    let write_result = write_body(res.body, &mut w, &mut meter);
    let lwip = LwipCounters::snapshot().since(lwip_before);
    let total_ms = started.elapsed().as_millis() as u64;
    note_httpd_stack_low_water();

    let phases = Phases {
        dispatch_ms,
        log_ms,
        hdr_ms,
        body_ms: total_ms
            .saturating_sub(dispatch_ms + log_ms + hdr_ms)
            .saturating_sub(meter.millis()),
        total_ms,
    };
    log_slow_exchange(method, uri, write_result.is_ok(), &phases, &meter, &lwip);
    write_result.map_err(|_| EspIOError::from(EspError::from_infallible::<ESP_FAIL>()))?;
    Ok(())
}

fn write_body<W>(body: HttpBody, w: &mut W, meter: &mut SocketMeter) -> io::Result<()>
where
    W: Write<Error = EspIOError>,
{
    let mut sink = MeteredWrite {
        inner: StdIoBridge(w),
        meter,
    };
    if RESPONSE_BUFFER_BYTES == 0 {
        body.write_to(&mut sink)
    } else {
        let mut buffered = io::BufWriter::with_capacity(RESPONSE_BUFFER_BYTES, sink);
        body.write_to(&mut buffered).and_then(|()| buffered.flush())
    }
}

fn log_slow_exchange(
    method: &str,
    uri: &str,
    write_ok: bool,
    p: &Phases,
    meter: &SocketMeter,
    lwip: &LwipCounters,
) {
    let socket_ms = meter.millis();
    if write_ok && p.total_ms <= SLOW_REQUEST_MS && socket_ms <= SLOW_SOCKET_MS {
        return;
    }
    let (free, largest_at_boot) = heap_free_and_largest_at_boot();
    let need = meter.bytes.div_ceil(TCP_MSS_BYTES);
    log::warn!(
        "HTTP slow/failed: {method} {uri} write_ok={write_ok} total_ms={} \
         [phase ms: dispatch={} log={} hdr={} body={} socket={socket_ms}] \
         [socket: {} B in {} writes] \
         [lwip: tcp_xmit={} need~{need} tcp_drop={} link_drop={} link_memerr={}] \
         [internal heap: free={free} largest={largest_at_boot}@boot] \
         [core: httpd={} dali_isr={:?}]",
        p.total_ms,
        p.dispatch_ms,
        p.log_ms,
        p.hdr_ms,
        p.body_ms,
        meter.bytes,
        meter.calls,
        lwip.tcp_xmit,
        lwip.tcp_drop,
        lwip.link_drop,
        lwip.link_memerr,
        HTTPD_CORE_ID.load(Ordering::Relaxed),
        crate::dali::transport::esp_idf::isr_core_id()
    );
}
