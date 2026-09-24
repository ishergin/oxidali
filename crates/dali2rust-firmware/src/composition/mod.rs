#[cfg(target_os = "espidf")]
mod esp_idf;

#[cfg(not(target_os = "espidf"))]
mod host;

#[cfg(target_os = "espidf")]
use esp_idf as platform;

#[cfg(not(target_os = "espidf"))]
use host as platform;

use dali2rust_adapters::{build_router_with_bus_and_transport, StackOptions};

use dali2rust_adapters::DaliRuntimeConfig;

#[cfg(target_os = "espidf")]
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_platform::slice_store::SliceStore;

pub const VERSION: &str = env!("DALI2RUST_VERSION");

const _: () = assert!(
    version_is_shaped(VERSION),
    "DALI2RUST_VERSION must be <x.y.z>+<identity>; see build.rs"
);

const fn version_is_shaped(v: &str) -> bool {
    let bytes = v.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            return i > 0 && i + 1 < bytes.len();
        }
        i += 1;
    }
    false
}

const HEARTBEAT_LOG_INTERVAL_SECS: u64 = 60;

#[cfg(target_os = "espidf")]
const INTERNAL_PRESSURE_ALERT_BYTES: u32 = 12 * 1024;

#[cfg(target_os = "espidf")]
static INTERNAL_MIN_REPORTED: AtomicU32 = AtomicU32::new(u32::MAX);

#[cfg(target_os = "espidf")]
pub(crate) const NO_IP_DISPLAY: &str = "no IP";

pub fn run() {
    dali2rust_bsp::alloc_probe::arm();
    apply_placement_knobs();

    #[allow(unused_variables, reason = "variables used conditionally per platform")]
    let (router, ws_hub, stack, maybe_ip, persistence_slices) = init_and_build();

    #[cfg(target_os = "espidf")]
    {
        if let Some(ref ip) = maybe_ip {
            let pr = stack.try_publish_ip_address_assigned(ip);
            if !matches!(pr, dali2rust_bus::PublishResult::Queued) {
                log::warn!("IP assigned event publish dropped: {pr:?}");
            }
        } else {
            log::warn!("No IP address available after boot");
        }
    }

    #[cfg(target_os = "espidf")]
    log::info!("dali2rust {VERSION}");
    log_stack_home();
    run_psram_atomic_selftest();
    #[cfg(target_os = "espidf")]
    dali2rust_adapters::ota::esp_idf::log_slots();
    #[cfg(target_os = "espidf")]
    let _ota_verifier = spawn_boot_verifier();

    // SAFETY: the bus stack lives for the whole process; its workers use channels it owns.
    let _stack = Box::leak(stack);
    #[cfg(target_os = "espidf")]
    spawn_ip_watcher(_stack);
    platform::mount_http(Arc::new(router), ws_hub);

    run_event_loop();
}

fn gated_persistence(slices: Option<Arc<dyn SliceStore>>) -> Option<Arc<dyn SliceStore>> {
    slices.filter(|_| {
        let enabled = persistence_enabled();
        if !enabled {
            log::warn!("registry persistence DISABLED: no persistence writes (no flash-cache-off windows) this build");
        }
        enabled
    })
}

#[cfg(target_os = "espidf")]
fn displayable_ip(ip: String) -> Option<String> {
    let valid = !ip.is_empty() && ip != NO_IP_DISPLAY;
    valid.then_some(ip)
}

fn init_and_build() -> (
    dali2rust_api::http::router::Router,
    Arc<dali2rust_adapters::WsHub>,
    Box<dali2rust_adapters::BusStackRuntime>,
    Option<String>,
    Option<Arc<dyn SliceStore>>,
) {
    #[cfg(target_os = "espidf")]
    let (hardware, ip, transport, _net, persistence_slices) = platform::init_boot();
    #[cfg(target_os = "espidf")]
    let runtime_config = platform::runtime_config_from_env();
    #[cfg(not(target_os = "espidf"))]
    let (hardware, _ip) = platform::init_boot();
    #[cfg(not(target_os = "espidf"))]
    let persistence_slices: Option<Arc<dyn SliceStore>> = None;
    #[cfg(not(target_os = "espidf"))]
    let transport = dali2rust_adapters::dali::transport::mock::MockDaliTransport::new();
    #[cfg(not(target_os = "espidf"))]
    let runtime_config = DaliRuntimeConfig::default();

    let wall_clock = Arc::new(dali2rust_bsp::wall_clock::SystemWallClock::new());
    #[cfg(target_os = "espidf")]
    dali2rust_bsp::sntp_sync::start(Arc::clone(&wall_clock));

    let persistence_slices = gated_persistence(persistence_slices);
    let transport = Arc::new(Mutex::new(transport));
    #[cfg(target_os = "espidf")]
    let maybe_ip = displayable_ip(ip);
    #[cfg(not(target_os = "espidf"))]
    let maybe_ip = None::<String>;

    let options = stack_options(runtime_config, persistence_slices.clone(), wall_clock);
    let (router, ws_hub, stack) =
        build_router_with_bus_and_transport(VERSION, transport, hardware, options);
    (router, ws_hub, stack, maybe_ip, persistence_slices)
}

fn stack_options(
    runtime_config: DaliRuntimeConfig,
    persistence_slices: Option<Arc<dyn SliceStore>>,
    wall_clock: Arc<dali2rust_bsp::wall_clock::SystemWallClock>,
) -> StackOptions {
    StackOptions {
        runtime_config,
        persistence_slices,
        heap_stats: heap_stats_port(),
        network_link: network_link(),
        wall_clock: Some(wall_clock),
        web_assets: crate::web_assets::WEB_ASSETS,
        controller_hardware_id: controller_hardware_id(),
        mqtt_client: mqtt_client(),
        firmware_update: firmware_update_ports(),
        ..StackOptions::default()
    }
}

#[cfg(target_os = "espidf")]
fn spawn_boot_verifier() -> Option<std::thread::JoinHandle<()>> {
    dali2rust_ota_runtime::spawn_boot_verifier(
        Arc::new(dali2rust_adapters::ota::esp_idf::EspFirmwarePort::default()),
        Arc::new(|| {
            platform::ETH_LINK
                .get()
                .is_some_and(|link| link.status().up)
        }),
    )
}

#[cfg(target_os = "espidf")]
fn firmware_update_ports() -> Option<dali2rust_adapters::ota::FirmwareUpdatePorts> {
    Some(dali2rust_adapters::ota::FirmwareUpdatePorts::esp())
}

#[cfg(not(target_os = "espidf"))]
fn firmware_update_ports() -> Option<dali2rust_adapters::ota::FirmwareUpdatePorts> {
    None
}

#[cfg(target_os = "espidf")]
fn mqtt_client() -> Option<dali2rust_platform::mqtt::MqttClientBundle> {
    Some(dali2rust_mqtt_runtime::EspMqttBridgeClient::bundle())
}

#[cfg(not(target_os = "espidf"))]
fn mqtt_client() -> Option<dali2rust_platform::mqtt::MqttClientBundle> {
    None
}

#[cfg(target_os = "espidf")]
fn controller_hardware_id() -> Option<[u8; 6]> {
    esp_idf::ETH_LINK.get().and_then(|l| l.hardware_address())
}

#[cfg(not(target_os = "espidf"))]
fn controller_hardware_id() -> Option<[u8; 6]> {
    None
}

#[cfg(target_os = "espidf")]
fn network_link() -> Option<Arc<dyn dali2rust_platform::net::NetworkLink>> {
    esp_idf::ETH_LINK.get().cloned()
}

#[cfg(not(target_os = "espidf"))]
fn network_link() -> Option<Arc<dyn dali2rust_platform::net::NetworkLink>> {
    None
}

#[cfg(target_os = "espidf")]
fn heap_stats_port() -> Option<Arc<dyn dali2rust_platform::heap::HeapStatsPort>> {
    Some(Arc::new(dali2rust_bsp::heap_stats::EspHeapStats))
}

#[cfg(not(target_os = "espidf"))]
fn heap_stats_port() -> Option<Arc<dyn dali2rust_platform::heap::HeapStatsPort>> {
    None
}

#[cfg(target_os = "espidf")]
fn log_heap_stats() {
    use dali2rust_platform::heap::HeapStatsPort;

    let h = dali2rust_bsp::heap_stats::EspHeapStats.snapshot();
    log::info!(
        "heap: free={} largest_free_block={}@boot min_free_ever={} | \
         internal: free={} largest={}@boot min_ever={}",
        h.free_bytes,
        h.largest_free_block_bytes,
        h.min_free_ever_bytes,
        h.internal_free_bytes,
        h.internal_largest_block_bytes,
        h.internal_min_free_bytes
    );
    log_rust_heap();
}

#[cfg(target_os = "espidf")]
fn log_rust_heap() {
    let r = dali2rust_bsp::rust_heap::figures();
    let [tiny, small, medium, large, huge, big] = r.internal_live_by_class;
    log::info!(
        "rust heap: internal live={} peak={} | psram live={} peak={} | \
         internal by size <64={tiny} <256={small} <1K={medium} <4K={large} <16K={huge} big={big}",
        r.internal_live_bytes,
        r.internal_peak_bytes,
        r.psram_live_bytes,
        r.psram_peak_bytes
    );
}

#[cfg(target_os = "espidf")]
fn log_core_placement() {
    log::info!(
        "cores: dali_isr={:?} httpd={:?} heartbeat={}",
        dali2rust_adapters::dali::transport::esp_idf::isr_core_id(),
        dali2rust_adapters::http::esp_idf::httpd_core_id(),
        esp_idf_svc::hal::cpu::core() as u32
    );
}

#[cfg(not(target_os = "espidf"))]
fn log_core_placement() {}

#[cfg(target_os = "espidf")]
fn log_network_link() {
    let Some(link) = platform::ETH_LINK.get() else {
        return;
    };
    let stats = link.stats();
    log::info!(
        "eth link: up={} rx={}pkt/{}B tx={}pkt/{}B up_events={}",
        link.status().up,
        stats.rx_packets,
        stats.rx_bytes,
        stats.tx_packets,
        stats.tx_bytes,
        stats.link_up_events
    );
    if stats.has_loss() {
        log::warn!(
            "eth link LOSS: rx_dropped={} tx_dropped={} rx_ring_overruns={} rx_fifo_overflows={}",
            stats.rx_dropped,
            stats.tx_dropped,
            stats.rx_ring_overruns,
            stats.rx_fifo_overflows
        );
    }
}

#[cfg(not(target_os = "espidf"))]
fn log_network_link() {}

#[cfg(target_os = "espidf")]
fn log_internal_pressure() {
    use esp_idf_svc::sys::{heap_caps_get_minimum_free_size, MALLOC_CAP_INTERNAL};

    // SAFETY: thread-safe ESP-IDF heap-caps runtime accessor.
    let min_ever = unsafe { heap_caps_get_minimum_free_size(MALLOC_CAP_INTERNAL) } as u32;
    if min_ever >= INTERNAL_PRESSURE_ALERT_BYTES
        || min_ever >= INTERNAL_MIN_REPORTED.load(Ordering::Relaxed)
    {
        return;
    }
    INTERNAL_MIN_REPORTED.store(min_ever, Ordering::Relaxed);
    let r = dali2rust_bsp::rust_heap::figures();
    log::warn!(
        "internal SRAM low-water: min_ever={min_ever} B | rust internal live={} peak={} psram live={}",
        r.internal_live_bytes,
        r.internal_peak_bytes,
        r.psram_live_bytes
    );
}

#[cfg(not(target_os = "espidf"))]
fn log_internal_pressure() {}

fn apply_placement_knobs() {
    if option_env!("DALI2RUST_STACKS_INTERNAL") == Some("1") {
        dali2rust_bsp::esp_thread::keep_stacks_internal();
    }
    if option_env!("DALI2RUST_RUST_HEAP_INTERNAL") == Some("1") {
        dali2rust_bsp::rust_heap::keep_rust_heap_internal();
    }
    #[cfg(target_os = "espidf")]
    if option_env!("DALI2RUST_ETH_RX_INTERNAL") == Some("1") {
        dali2rust_bsp::esp32p4::eth::keep_rx_frames_internal();
    }
}

#[cfg(target_os = "espidf")]
fn log_stack_home() {
    let home = if dali2rust_bsp::esp_thread::external_stacks_on_xip() {
        "PSRAM"
    } else {
        "internal SRAM"
    };
    log::warn!("worker and httpd stacks: {home}");
    if dali2rust_bsp::rust_heap::psram_first_active() {
        log::warn!(
            "rust heap: objects from {} B in PSRAM",
            dali2rust_bsp::rust_heap::PSRAM_FIRST_FROM_BYTES
        );
    } else {
        log::warn!("rust heap: internal SRAM first");
    }
    let rx = if dali2rust_bsp::esp32p4::eth::rx_frames_in_psram() {
        "PSRAM"
    } else {
        "internal SRAM"
    };
    log::warn!("eth received frames: {rx}");
}

#[cfg(not(target_os = "espidf"))]
fn log_stack_home() {}

#[cfg(target_os = "espidf")]
fn run_psram_atomic_selftest() {
    if option_env!("DALI2RUST_PSRAM_ATOMIC_SELFTEST") != Some("1") {
        return;
    }
    match dali2rust_bsp::psram_selftest::run() {
        Some(t) if t.passed() => log::warn!(
            "psram atomics self-test: ok, {} increments of each kind, {} us",
            t.expected,
            t.elapsed_us
        ),
        Some(t) => log::error!("psram atomics self-test: FAILED {t:?}"),
        None => log::error!("psram atomics self-test: no PSRAM block"),
    }
}

#[cfg(not(target_os = "espidf"))]
fn run_psram_atomic_selftest() {}

fn persistence_enabled() -> bool {
    match option_env!("DALI2RUST_PERSIST_DISABLE") {
        None => true,
        Some(raw) => raw.is_empty() || raw == "0",
    }
}

fn heap_check_period_s() -> u64 {
    option_env!("DALI2RUST_HEAP_CHECK_S")
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(0)
}

#[cfg(target_os = "espidf")]
fn check_heap_integrity() {
    // SAFETY: thread-safe ESP-IDF heap walker; `true` prints corrupt blocks.
    let ok = unsafe { esp_idf_svc::sys::heap_caps_check_integrity_all(true) };
    if !ok {
        panic!("heap integrity check FAILED (see printed corrupt blocks)");
    }
}

#[cfg(not(target_os = "espidf"))]
fn check_heap_integrity() {}

#[cfg(target_os = "espidf")]
fn log_task_stack_census() {
    use core::fmt::Write;
    const KERNEL_TASKS: &[&core::ffi::CStr] = &[
        c"IDLE0",
        c"IDLE1",
        c"ipc0",
        c"ipc1",
        c"esp_timer",
        c"Tmr Svc",
        c"tiT",
        c"sys_evt",
    ];
    dali2rust_bsp::task_registry::refresh_kernel_tasks(KERNEL_TASKS);
    let mut rows = Vec::new();
    dali2rust_bsp::task_registry::snapshot_stack_free(&mut rows);
    let mut line = String::new();
    let mut n = rows.len();
    for (name, free) in &rows {
        let _ = write!(line, "{}={free} ", name.to_string_lossy());
    }
    n += write_observed_task_census(&mut line);
    if n == 0 {
        log::warn!("task stack hwm: MEASURED NOTHING — the task registry is empty");
        return;
    }
    log::info!("task stack hwm (B free, {n} tasks): {line}");
}

#[cfg(target_os = "espidf")]
fn write_observed_task_census(line: &mut String) -> usize {
    use core::fmt::Write;
    let mut count = 0;
    let _ = write!(line, "| observed: ");
    dali2rust_bsp::task_registry::for_each_observed(|sample, name| match sample {
        Some(sample) => {
            count += 1;
            let freshness = if sample.fresh { "fresh" } else { "stale" };
            let _ = write!(
                line,
                "{}={}@0x{:x}@{}:{} ",
                sample.name.to_string_lossy(),
                sample.free,
                sample.instance,
                sample.age_ms,
                freshness
            );
        }
        None => {
            let _ = write!(line, "{}=missing ", name.to_string_lossy());
        }
    });
    count
}

fn run_event_loop() {
    log::info!("Firmware initialized");
    dali2rust_bsp::alloc_probe::log_armed();
    let heap_check_s = heap_check_period_s();
    if heap_check_s > 0 {
        log::warn!("heap integrity sweeps armed: every {heap_check_s}s");
    }
    if !spawn_heartbeat_thread(heap_check_s) {
        heartbeat_loop(heap_check_s);
    }
}

fn heartbeat_loop(heap_check_s: u64) -> ! {
    let mut tick: u64 = 0;
    loop {
        // sleep-ok: heartbeat tick, once a second
        std::thread::sleep(Duration::from_secs(1));
        tick = tick.wrapping_add(1);
        heartbeat_tick(tick, heap_check_s);
    }
}

fn heartbeat_tick(tick: u64, heap_check_s: u64) {
    dali2rust_bsp::alloc_probe::poll(false);
    log_internal_pressure();
    if heap_check_s > 0 && tick % heap_check_s == 0 {
        check_heap_integrity();
    }
    if tick % HEARTBEAT_LOG_INTERVAL_SECS == 0 {
        log::info!("firmware heartbeat: uptime={tick}s");
        log_console_drops();
        log_core_placement();
        log_network_link();
        dali2rust_bsp::alloc_probe::poll(true);
        log_census();
    }
}

#[cfg(target_os = "espidf")]
fn log_console_drops() {
    let dropped = dali2rust_adapters::log::console::dropped();
    let busy = dali2rust_adapters::log::console::busy();
    let truncated = dali2rust_adapters::log::console::truncated();
    let unavailable = dali2rust_adapters::log::console::unavailable();
    let uart_errors = dali2rust_adapters::log::console::uart_errors();
    if dropped + truncated + uart_errors > 0 {
        log::warn!(
            "console: dropped={dropped} (busy={busy} unavailable={unavailable}) truncated={truncated} uart_errors={uart_errors}"
        );
    }
}

#[cfg(not(target_os = "espidf"))]
fn log_console_drops() {}

#[cfg(target_os = "espidf")]
fn spawn_heartbeat_thread(heap_check_s: u64) -> bool {
    const CORE_AWAY_FROM_THE_INTERRUPT: u8 = 1;
    let spawned = dali2rust_bsp::esp_thread::try_spawn_named_stack_in(
        c"census",
        dali2rust_bsp::std_thread_stack::CENSUS_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        Some(CORE_AWAY_FROM_THE_INTERRUPT),
        move || heartbeat_loop(heap_check_s),
    );
    if let Err(e) = &spawned {
        log::error!("heartbeat thread failed to spawn: {e:?} — the heartbeat stays on main");
    }
    spawned.is_ok()
}

#[cfg(not(target_os = "espidf"))]
fn spawn_heartbeat_thread(_heap_check_s: u64) -> bool {
    false
}

#[cfg(target_os = "espidf")]
fn log_census() {
    // SAFETY: monotonic µs clock, callable from any task.
    let t0 = unsafe { esp_idf_svc::sys::esp_timer_get_time() };
    log_heap_stats();
    // SAFETY: as above.
    let t1 = unsafe { esp_idf_svc::sys::esp_timer_get_time() };
    log_task_stack_census();
    // SAFETY: as above.
    let t2 = unsafe { esp_idf_svc::sys::esp_timer_get_time() };
    log::info!(
        "census timing: heap stats {} us, task census {} us (core {})",
        t1 - t0,
        t2 - t1,
        esp_idf_svc::hal::cpu::core() as u32
    );
}

#[cfg(not(target_os = "espidf"))]
fn log_census() {}

#[cfg(target_os = "espidf")]
fn spawn_ip_watcher(stack: &'static dali2rust_adapters::BusStackRuntime) {
    let Some(link) = platform::ETH_LINK.get().cloned() else {
        log::warn!("ip watcher: no link handle — display will keep the boot value");
        return;
    };
    const POLL: Duration = Duration::from_secs(2);
    let spawned = dali2rust_bsp::esp_thread::try_spawn_named_stack_in(
        c"ip-watch",
        dali2rust_bsp::std_thread_stack::RING_CONSUMER_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        None,
        move || {
            let mut published: Option<Option<[u8; 4]>> = Some(link.status().ipv4);
            loop {
                // sleep-ok: slow poll of an external event, far above the floor.
                std::thread::sleep(POLL);
                let now = link.status().ipv4;
                if published == Some(now) {
                    continue;
                }
                let ip = match now {
                    Some(o) => format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3]),
                    None => NO_IP_DISPLAY.to_string(),
                };
                log::info!("eth: address now {ip} — publishing to the display");
                let pr = stack.try_publish_ip_address_assigned(&ip);
                if matches!(pr, dali2rust_bus::PublishResult::Queued) {
                    published = Some(now);
                } else {
                    log::warn!("ip watcher: publish dropped ({pr:?}) — retrying next poll");
                }
            }
        },
    );
    if spawned.is_err() {
        log::error!("ip watcher: thread spawn failed");
    }
}
