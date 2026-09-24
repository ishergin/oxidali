use dali2rust_adapters::dali::transport::esp_idf::EspIdfDaliTransport;
use dali2rust_adapters::display::esp_idf::Ssd1306Display;
use dali2rust_adapters::{ContentConfirmPolicy, DaliRuntimeConfig, HardwareDisplay};
use dali2rust_api::http::router::Router;
use dali2rust_domain::dali::ses::RetryPolicy;
use dali2rust_platform::slice_store::SliceStore;

use std::sync::Arc;
use std::sync::Mutex;

use dali2rust_platform::display::DisplayDriver;

use dali2rust_platform::net::NetworkLink;

const PROJECT_LOG_TARGETS: &[&str] = &[
    "dali2rust_adapters::dali::transport::esp_idf",
    "dali2rust_adapters::dali::transport::phy_interrupt",
    "dali2rust_adapters::http::esp_idf",
    "dali2rust_adapters::http::wire_method",
    "dali2rust_adapters::runtime::registry_init",
    "dali2rust_api::confirmation_bridge",
    "dali2rust_api::http::handlers::common",
    "dali2rust_api::http::handlers::operations",
    "dali2rust_bsp::esp32p4::eth",
    "dali2rust_bsp::esp_thread",
    "dali2rust_bsp::slice_store_raw",
    "dali2rust_dali_runtime::runtime::dali_worker",
    "dali2rust_display_runtime::display::esp_idf",
    "dali2rust_firmware::composition",
    "dali2rust_firmware::composition::esp_idf",
    "dali2rust_firmware::composition::host",
    "dali2rust_firmware::persistence_store",
];

macro_rules! parse_env_or {
    ($name:literal, $default:expr) => {
        option_env!($name)
            .and_then(|value| value.parse().ok())
            .unwrap_or($default)
    };
}

static HTTPD_STACK_RESERVE: Mutex<Option<dali2rust_adapters::http::esp_idf::HttpdStackReserve>> =
    Mutex::new(None);

pub fn init_boot() -> (
    HardwareDisplay,
    String,
    EspIdfDaliTransport,
    Arc<dyn NetworkLink>,
    Option<Arc<dyn SliceStore>>,
) {
    let peripherals = boot_prologue();
    let (link, ip) = setup_eth();

    let persistence_slices = init_persistence_store();
    let dali_transport = setup_dali_transport(peripherals.pins.gpio14, peripherals.pins.gpio17);
    let hardware = init_display_oled(
        peripherals.i2c0,
        peripherals.pins.gpio33,
        peripherals.pins.gpio32,
    );
    (hardware, ip, dali_transport, link, persistence_slices)
}

fn boot_prologue() -> esp_idf_svc::hal::peripherals::Peripherals {
    esp_idf_svc::sys::link_patches();
    init_logging();
    log_flash_identity();
    *HTTPD_STACK_RESERVE.lock().unwrap() =
        Some(dali2rust_adapters::http::esp_idf::HttpdStackReserve::take());
    take_peripherals()
}

const GD_SUSPEND_CAPABLE: [u32; 4] = [0xC8_4016, 0xC8_4017, 0xC8_4018, 0xC8_4319];

fn log_flash_identity() {
    let mut id: u32 = 0;
    // SAFETY: startup initialises `esp_flash_default_chip` before `app_main`; `esp_flash_read_id` locks the flash.
    let err = unsafe {
        esp_idf_svc::sys::esp_flash_read_id(esp_idf_svc::sys::esp_flash_default_chip, &mut id)
    };
    if err != esp_idf_svc::sys::ESP_OK {
        log::warn!("flash: jedec id unreadable (esp_err {err})");
        return;
    }
    let suspend = if GD_SUSPEND_CAPABLE.contains(&id) { "yes" } else { "no (gd table)" };
    log::info!("flash: jedec=0x{id:06x} erase-suspend capable={suspend}");
}

pub static ETH_LINK: std::sync::OnceLock<Arc<dyn NetworkLink>> = std::sync::OnceLock::new();

fn setup_eth() -> (Arc<dyn NetworkLink>, String) {
    match dali2rust_bsp::esp32p4::eth::start() {
        Ok(link) => {
            let link: Arc<dyn NetworkLink> = Arc::new(link);
            let _ = ETH_LINK.set(Arc::clone(&link));
            let ip = current_ip(link.as_ref());
            (link, ip)
        }
        Err(e) => {
            log::error!("eth: bring-up failed: {e:?} — continuing without a network");
            (Arc::new(NoLink), String::from(super::NO_IP_DISPLAY))
        }
    }
}

fn current_ip(link: &dyn NetworkLink) -> String {
    match link.status().ipv4 {
        Some(o) => {
            let ip = format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3]);
            log::info!("eth: DHCP already bound {ip}");
            ip
        }
        None => {
            log::info!("eth: no lease yet — the IP watcher publishes it when it lands");
            String::from(super::NO_IP_DISPLAY)
        }
    }
}

#[derive(Debug, Default)]
struct NoLink;

impl NetworkLink for NoLink {
    fn kind(&self) -> &'static str {
        "none"
    }
    fn status(&self) -> dali2rust_platform::net::LinkStatus {
        Default::default()
    }
    fn stats(&self) -> dali2rust_platform::net::LinkStats {
        Default::default()
    }
}

pub fn runtime_config_from_env() -> DaliRuntimeConfig {
    DaliRuntimeConfig::new(
        retry_policy_from_env(),
        parse_env_or!(
            "DALI2RUST_DALI_DISCOVERY_STEP_RETRIES",
            DaliRuntimeConfig::default().discovery_step_retries
        ),
        content_confirm_from_env(),
    )
    .with_target_state_sequence_retries(parse_env_or!(
        "DALI2RUST_DALI_TARGET_SEQUENCE_RETRIES",
        DaliRuntimeConfig::default().target_state_sequence_retries
    ))
}

fn retry_policy_from_env() -> RetryPolicy {
    RetryPolicy::new(
        parse_env_or!(
            "DALI2RUST_DALI_RETRY_MAX_ATTEMPTS",
            RetryPolicy::default().max_attempts
        ),
        parse_env_or!(
            "DALI2RUST_DALI_RETRY_BACKOFF_MS",
            RetryPolicy::default().base_backoff_ms
        ),
        parse_env_or!(
            "DALI2RUST_DALI_RETRY_JITTER_MS",
            RetryPolicy::default().jitter_ms
        ),
    )
    .with_query_contention_retry(parse_env_or!(
        "DALI2RUST_DALI_QUERY_CONTENTION_RETRY",
        RetryPolicy::default().query_contention_retry
    ))
}

fn content_confirm_from_env() -> ContentConfirmPolicy {
    ContentConfirmPolicy::new(
        parse_env_or!(
            "DALI2RUST_DALI_QUERY_CONTENT_CONFIRM",
            DaliRuntimeConfig::default().content_confirm.enabled
        ),
        parse_env_or!(
            "DALI2RUST_DALI_QUERY_CONTENT_CONFIRM_MAX_SAMPLES",
            DaliRuntimeConfig::default().content_confirm.max_samples
        ),
    )
}

fn init_logging() {
    if !dali2rust_adapters::log::console::install() {
        init_logging_without_strict_sink();
        return;
    }
    let logger = dali2rust_adapters::log::esp_idf::install_facade(PROJECT_LOG_TARGETS);
    dali2rust_adapters::log::esp_idf::install();
    install_panic_drain();
    if option_env!("DALI2RUST_ESP_VERBOSE").is_some() {
        set_target_levels(logger, &["*"], log::LevelFilter::Trace);
        log::warn!("logging: verbose ESP-IDF component logs enabled via DALI2RUST_ESP_VERBOSE");
        return;
    }
    set_target_levels(logger, &["*"], log::LevelFilter::Warn);
    set_target_levels(logger, PROJECT_LOG_TARGETS, log::LevelFilter::Trace);
}

fn install_panic_drain() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        dali2rust_adapters::log::console::drain_to_rom_console();
        previous(info);
    }));
}

fn init_logging_without_strict_sink() {
    let logger = esp_idf_svc::log::init_from_esp_idf();
    set_target_levels(logger, &["*"], log::LevelFilter::Warn);
    set_target_levels(logger, PROJECT_LOG_TARGETS, log::LevelFilter::Trace);
    log::error!(
        "logging: strict console sink unavailable; ROM console retained, no hook, I9 history has no source"
    );
}

fn set_target_levels(
    logger: &esp_idf_svc::log::EspLogger,
    targets: &[&str],
    level: log::LevelFilter,
) {
    for &target in targets {
        if let Err(e) = logger.filter().set_target_level(target, level) {
            log::warn!("logging: set level for {target} failed: {e:?}");
        }
    }
}

fn init_persistence_store() -> Option<Arc<dyn SliceStore>> {
    crate::persistence_store::mount_persistence_store()
}

fn take_peripherals() -> esp_idf_svc::hal::peripherals::Peripherals {
    match esp_idf_svc::hal::peripherals::Peripherals::take() {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to take peripherals: {e:?}");
            std::process::abort();
        }
    }
}

fn setup_dali_transport<Tx, Rx>(tx_pin: Tx, rx_pin: Rx) -> EspIdfDaliTransport
where
    Tx: esp_idf_svc::hal::gpio::OutputPin + esp_idf_svc::hal::gpio::Pin + 'static,
    Rx: esp_idf_svc::hal::gpio::InputPin + esp_idf_svc::hal::gpio::Pin + 'static,
{
    const _: () = assert!(dali2rust_bsp::esp32p4::pins::DALI_TX_GPIO == 14);
    const _: () = assert!(dali2rust_bsp::esp32p4::pins::DALI_RX_GPIO == 17);
    let level = dali2rust_adapters::dali::transport::phy_interrupt::PhyIsrLevel::from_knob(
        option_env!("DALI2RUST_PHY_ISR_LEVEL"),
    );
    match EspIdfDaliTransport::try_new_with_level(tx_pin, rx_pin, level) {
        Ok(t) => {
            log::info!(
                "DALI: GPTimer bit-bang PHY initialized (TX GPIO{}, RX GPIO{})",
                dali2rust_bsp::board_pins::DALI_TX_GPIO,
                dali2rust_bsp::board_pins::DALI_RX_GPIO,
            );
            t
        }
        Err(e) => {
            log::error!("DALI: GPTimer bit-bang PHY init failed ({e:?})");
            std::process::abort();
        }
    }
}

fn scan_i2c_bus(i2c: &mut esp_idf_svc::hal::i2c::I2cDriver<'_>) {
    let mut found = 0u8;
    for addr in 0x08u8..0x78 {
        if i2c.write(addr, &[], 100).is_ok() {
            log::warn!("I2C scan: device ACKs at 0x{addr:02x}");
            found += 1;
        }
    }
    if found == 0 {
        log::warn!("I2C scan: no devices found (check wiring/power)");
    }
}

fn init_display_oled<Sda, Scl>(
    i2c0: esp_idf_svc::hal::i2c::I2C0<'static>,
    sda: Sda,
    scl: Scl,
) -> HardwareDisplay
where
    Sda: esp_idf_svc::hal::gpio::InputPin + esp_idf_svc::hal::gpio::OutputPin + 'static,
    Scl: esp_idf_svc::hal::gpio::InputPin + esp_idf_svc::hal::gpio::OutputPin + 'static,
{
    let mut i2c = match dali2rust_bsp::i2c::init_i2c(i2c0, sda, scl) {
        Ok(d) => d,
        Err(e) => {
            log::error!("I2C init failed: {e:?}");
            return HardwareDisplay::none();
        }
    };
    scan_i2c_bus(&mut i2c);

    let mut display = Ssd1306Display::new(i2c);

    match display.init() {
        Ok(()) => log::info!("Display: initialized"),
        Err(e) => {
            log::error!("Display init failed: {e:?}");
            return HardwareDisplay::none();
        }
    }
    let _ = display.clear();
    if let Err(e) = display.flush() {
        log::error!("Display flush failed: {e:?}");
        return HardwareDisplay::none();
    }

    HardwareDisplay::EspOled(Arc::new(Mutex::new(display)))
}

pub fn mount_http(router: Arc<Router>, ws_hub: Arc<dali2rust_adapters::WsHub>) {
    let reserve = HTTPD_STACK_RESERVE
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(dali2rust_adapters::http::esp_idf::HttpdStackReserve::take);
    match dali2rust_adapters::http::esp_idf::mount(router, ws_hub, reserve) {
        Ok(server) => {
            log::info!("HTTP server listening on port 80");
            // SAFETY: HTTP server must live for the entire process lifetime.
            Box::leak(Box::new(server));
        }
        Err(e) => {
            log::error!("HTTP server failed: {e:?}");
            std::process::abort();
        }
    }
}
