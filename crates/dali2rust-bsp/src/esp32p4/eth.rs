use core::ffi::c_void;
use core::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use esp_idf_svc::sys::{self, esp, EspError};

use dali2rust_platform::net::{LinkStats, LinkStatus, NetworkLink};

use super::emac_dma::{decode_missed_frames, MISSED_FRAMES_WORD};
use super::pins;

const PHY_ADDR_AUTO: i32 = -1;

const PHY_RESET_TIMEOUT_MS: u32 = 100;
const PHY_AUTONEG_TIMEOUT_MS: u32 = 4_000;

const EMAC_RX_TASK_STACK: u32 = 4_096;
const EMAC_RX_TASK_PRIO: u32 = 15;

static COUNTERS: Counters = Counters::new();

static RX_FRAMES_IN_PSRAM: AtomicBool = AtomicBool::new(true);

pub fn keep_rx_frames_internal() {
    RX_FRAMES_IN_PSRAM.store(false, Ordering::Relaxed);
}

pub fn rx_frames_in_psram() -> bool {
    RX_FRAMES_IN_PSRAM.load(Ordering::Relaxed)
}

unsafe extern "C" {
    static EMAC_DMA: u32;
}

struct Counters {
    rx_packets: AtomicU32,
    tx_packets: AtomicU32,
    rx_bytes: AtomicU32,
    tx_bytes: AtomicU32,
    rx_dropped: AtomicU32,
    tx_dropped: AtomicU32,
    rx_ring_overruns: AtomicU32,
    rx_fifo_overflows: AtomicU32,
    link_up_events: AtomicU32,
}

impl Counters {
    const fn new() -> Self {
        Self {
            rx_packets: AtomicU32::new(0),
            tx_packets: AtomicU32::new(0),
            rx_bytes: AtomicU32::new(0),
            tx_bytes: AtomicU32::new(0),
            rx_dropped: AtomicU32::new(0),
            tx_dropped: AtomicU32::new(0),
            rx_ring_overruns: AtomicU32::new(0),
            rx_fifo_overflows: AtomicU32::new(0),
            link_up_events: AtomicU32::new(0),
        }
    }

    fn add(counter: &AtomicU32, n: u32) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    fn take_missed_frames(&self) {
        // SAFETY: `EMAC_DMA` is ESP-IDF's linker symbol for the EMAC DMA block; the word is read-to-clear and only read here.
        let raw = unsafe { ptr::read_volatile(ptr::addr_of!(EMAC_DMA).add(MISSED_FRAMES_WORD)) };
        let missed = decode_missed_frames(raw);
        Self::add(&self.rx_ring_overruns, missed.no_descriptor);
        Self::add(&self.rx_fifo_overflows, missed.fifo_overflow);
    }
}

unsafe extern "C" fn eth_input_to_netif(
    _hdl: sys::esp_eth_handle_t,
    buffer: *mut u8,
    length: u32,
    priv_: *mut c_void,
) -> sys::esp_err_t {
    Counters::add(&COUNTERS.rx_packets, 1);
    Counters::add(&COUNTERS.rx_bytes, length);
    let frame = if rx_frames_in_psram() {
        move_frame_to_psram(buffer, length as usize)
    } else {
        buffer
    };
    let err = sys::esp_netif_receive(
        priv_ as *mut sys::esp_netif_t,
        frame as *mut c_void,
        length as usize,
        ptr::null_mut(),
    );
    if err != sys::ESP_OK {
        Counters::add(&COUNTERS.rx_dropped, 1);
    }
    err
}

unsafe fn move_frame_to_psram(buffer: *mut u8, length: usize) -> *mut u8 {
    let copy = sys::heap_caps_malloc(length, sys::MALLOC_CAP_SPIRAM | sys::MALLOC_CAP_8BIT).cast::<u8>();
    if copy.is_null() {
        return buffer;
    }
    ptr::copy_nonoverlapping(buffer, copy, length);
    sys::free(buffer.cast());
    copy
}

unsafe extern "C" fn eth_transmit(
    hdl: *mut c_void,
    buffer: *mut c_void,
    len: usize,
) -> sys::esp_err_t {
    Counters::add(&COUNTERS.tx_packets, 1);
    Counters::add(&COUNTERS.tx_bytes, len as u32);
    let err = sys::esp_eth_transmit(hdl as sys::esp_eth_handle_t, buffer, len);
    if err != sys::ESP_OK {
        Counters::add(&COUNTERS.tx_dropped, 1);
    }
    err
}

unsafe extern "C" fn eth_free_rx_buffer(_h: *mut c_void, buffer: *mut c_void) {
    sys::free(buffer);
}

pub struct EthLink {
    netif: *mut sys::esp_netif_t,
    handle: sys::esp_eth_handle_t,
    mac: [u8; 6],
}

// SAFETY: both pointers are opaque ESP-IDF handles with thread-safe APIs; this type only reads through them.
unsafe impl Send for EthLink {}
unsafe impl Sync for EthLink {}

impl NetworkLink for EthLink {
    fn kind(&self) -> &'static str {
        "eth"
    }

    fn status(&self) -> LinkStatus {
        let up = self.link_is_up();
        LinkStatus {
            up,
            speed_mbps: if up { self.negotiated_speed_mbps() } else { 0 },
            full_duplex: up && self.negotiated_full_duplex(),
            ipv4: self.ipv4(),
        }
    }

    fn hardware_address(&self) -> Option<[u8; 6]> {
        Some(self.mac)
    }

    fn stats(&self) -> LinkStats {
        COUNTERS.take_missed_frames();
        LinkStats {
            rx_packets: COUNTERS.rx_packets.load(Ordering::Relaxed),
            tx_packets: COUNTERS.tx_packets.load(Ordering::Relaxed),
            rx_bytes: COUNTERS.rx_bytes.load(Ordering::Relaxed),
            tx_bytes: COUNTERS.tx_bytes.load(Ordering::Relaxed),
            rx_dropped: COUNTERS.rx_dropped.load(Ordering::Relaxed),
            tx_dropped: COUNTERS.tx_dropped.load(Ordering::Relaxed),
            rx_ring_overruns: COUNTERS.rx_ring_overruns.load(Ordering::Relaxed),
            rx_fifo_overflows: COUNTERS.rx_fifo_overflows.load(Ordering::Relaxed),
            link_up_events: COUNTERS.link_up_events.load(Ordering::Relaxed),
        }
    }
}

impl EthLink {
    fn link_is_up(&self) -> bool {
        // SAFETY: `netif` is a live handle for the lifetime of the firmware.
        unsafe { sys::esp_netif_is_netif_up(self.netif) }
    }

    fn negotiated_speed_mbps(&self) -> u16 {
        match self.phy_ioctl(sys::esp_eth_io_cmd_t_ETH_CMD_G_SPEED) {
            Some(sys::eth_speed_t_ETH_SPEED_100M) => 100,
            Some(sys::eth_speed_t_ETH_SPEED_10M) => 10,
            _ => 0,
        }
    }

    fn negotiated_full_duplex(&self) -> bool {
        self.phy_ioctl(sys::esp_eth_io_cmd_t_ETH_CMD_G_DUPLEX_MODE)
            == Some(sys::eth_duplex_t_ETH_DUPLEX_FULL)
    }

    fn phy_ioctl(&self, cmd: sys::esp_eth_io_cmd_t) -> Option<u32> {
        let mut out: u32 = 0;
        // SAFETY: `out` is a live u32 and every command used here writes one.
        let err = unsafe {
            sys::esp_eth_ioctl(self.handle, cmd, &mut out as *mut u32 as *mut c_void)
        };
        (err == sys::ESP_OK).then_some(out)
    }

    fn ipv4(&self) -> Option<[u8; 4]> {
        let mut info = sys::esp_netif_ip_info_t::default();
        // SAFETY: `info` is a live, correctly typed out-parameter.
        let err = unsafe { sys::esp_netif_get_ip_info(self.netif, &mut info) };
        if err != sys::ESP_OK || info.ip.addr == 0 {
            return None;
        }
        Some(info.ip.addr.to_le_bytes())
    }
}

unsafe extern "C" fn on_eth_event(
    arg: *mut c_void,
    _base: sys::esp_event_base_t,
    event_id: i32,
    _data: *mut c_void,
) {
    let netif = arg as *mut sys::esp_netif_t;
    match event_id as u32 {
        sys::eth_event_t_ETHERNET_EVENT_START => {
            sys::esp_netif_action_start(netif as *mut c_void, ptr::null(), 0, ptr::null_mut());
        }
        sys::eth_event_t_ETHERNET_EVENT_STOP => {
            sys::esp_netif_action_stop(netif as *mut c_void, ptr::null(), 0, ptr::null_mut());
        }
        sys::eth_event_t_ETHERNET_EVENT_CONNECTED => {
            Counters::add(&COUNTERS.link_up_events, 1);
            sys::esp_netif_action_connected(netif as *mut c_void, ptr::null(), 0, ptr::null_mut());
        }
        sys::eth_event_t_ETHERNET_EVENT_DISCONNECTED => {
            sys::esp_netif_action_disconnected(netif as *mut c_void, ptr::null(), 0, ptr::null_mut());
        }
        _ => {}
    }
}

fn emac_config() -> sys::eth_esp32_emac_config_t {
    let mut cfg = sys::eth_esp32_emac_config_t::default();
    cfg.__bindgen_anon_1.smi_gpio = sys::emac_esp_smi_gpio_config_t {
        mdc_num: pins::ETH_MDC_GPIO as i32,
        mdio_num: pins::ETH_MDIO_GPIO as i32,
    };
    cfg.interface = sys::eth_data_interface_t_EMAC_DATA_INTERFACE_RMII;
    cfg.clock_config.rmii.clock_mode = sys::emac_rmii_clock_mode_t_EMAC_CLK_EXT_IN;
    cfg.clock_config.rmii.clock_gpio = pins::ETH_REF_CLK_GPIO as i32;
    cfg.dma_burst_len = sys::eth_mac_dma_burst_len_t_ETH_DMA_BURST_LEN_32;
    cfg.emac_dataif_gpio.rmii = sys::eth_mac_rmii_gpio_config_t {
        tx_en_num: pins::ETH_TX_EN_GPIO as i32,
        txd0_num: pins::ETH_TXD0_GPIO as i32,
        txd1_num: pins::ETH_TXD1_GPIO as i32,
        crs_dv_num: pins::ETH_CRS_DV_GPIO as i32,
        rxd0_num: pins::ETH_RXD0_GPIO as i32,
        rxd1_num: pins::ETH_RXD1_GPIO as i32,
    };
    cfg
}

fn mac_config() -> sys::eth_mac_config_t {
    let mut cfg = sys::eth_mac_config_t::default();
    cfg.sw_reset_timeout_ms = 100;
    cfg.rx_task_stack_size = EMAC_RX_TASK_STACK;
    cfg.rx_task_prio = EMAC_RX_TASK_PRIO;
    cfg.flags = 0;
    cfg
}

fn phy_config() -> sys::eth_phy_config_t {
    let mut cfg = sys::eth_phy_config_t::default();
    cfg.phy_addr = PHY_ADDR_AUTO;
    cfg.reset_timeout_ms = PHY_RESET_TIMEOUT_MS;
    cfg.autonego_timeout_ms = PHY_AUTONEG_TIMEOUT_MS;
    cfg.reset_gpio_num = pins::ETH_PHY_RESET_GPIO as i32;
    cfg
}

fn install_driver() -> Result<sys::esp_eth_handle_t, EspError> {
    let esp32_cfg = emac_config();
    let mac_cfg = mac_config();
    let phy_cfg = phy_config();
    // SAFETY: the three configs outlive the calls; the driver owns the returned objects from install on.
    unsafe {
        let mac = sys::esp_eth_mac_new_esp32(&esp32_cfg, &mac_cfg);
        let phy = sys::esp_eth_phy_new_ip101(&phy_cfg);
        let mut cfg = sys::esp_eth_config_t::default();
        cfg.mac = mac;
        cfg.phy = phy;
        cfg.check_link_period_ms = 2_000;
        let mut handle: sys::esp_eth_handle_t = ptr::null_mut();
        esp!(sys::esp_eth_driver_install(&cfg, &mut handle))?;
        Ok(handle)
    }
}

fn attach_counting_glue(
    handle: sys::esp_eth_handle_t,
) -> Result<(*mut sys::esp_netif_t, [u8; 6]), EspError> {
    // SAFETY: ESP-IDF's own default Ethernet netif statics; `esp_netif_set_driver_config` copies the config.
    unsafe {
        let mut cfg = sys::esp_netif_config_t {
            base: &sys::_g_esp_netif_inherent_eth_config,
            driver: ptr::null(),
            stack: sys::_g_esp_netif_netstack_default_eth,
        };
        let netif = sys::esp_netif_new(&mut cfg);
        if netif.is_null() {
            return Err(EspError::from_infallible::<{ sys::ESP_ERR_NO_MEM }>());
        }
        let driver_cfg = sys::esp_netif_driver_ifconfig_t {
            handle: handle as *mut c_void,
            transmit: Some(eth_transmit),
            transmit_wrap: None,
            driver_free_rx_buffer: Some(eth_free_rx_buffer),
            driver_set_mac_filter: None,
        };
        esp!(sys::esp_netif_set_driver_config(netif, &driver_cfg))?;
        esp!(sys::esp_eth_update_input_path(
            handle,
            Some(eth_input_to_netif),
            netif as *mut c_void
        ))?;
        let mut mac = [0u8; 6];
        esp!(sys::esp_eth_ioctl(
            handle,
            sys::esp_eth_io_cmd_t_ETH_CMD_G_MAC_ADDR,
            mac.as_mut_ptr() as *mut c_void
        ))?;
        esp!(sys::esp_netif_set_mac(netif, mac.as_mut_ptr()))?;
        log::info!(
            "eth: mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
        Ok((netif, mac))
    }
}

pub fn start() -> Result<EthLink, EspError> {
    // SAFETY: one-time stack init, before any netif exists.
    unsafe {
        esp!(sys::esp_netif_init())?;
        let err = sys::esp_event_loop_create_default();
        if err != sys::ESP_OK && err != sys::ESP_ERR_INVALID_STATE {
            esp!(err)?;
        }
    }

    let handle = install_driver()?;
    let (netif, mac) = attach_counting_glue(handle)?;

    // SAFETY: `netif` outlives the handler; it is never freed.
    unsafe {
        esp!(sys::esp_event_handler_register(
            sys::ETH_EVENT,
            sys::ESP_EVENT_ANY_ID,
            Some(on_eth_event),
            netif as *mut c_void
        ))?;
        esp!(sys::esp_eth_start(handle))?;
    }

    log::info!(
        "eth: EMAC started — mdc={} mdio={} ref_clk={} reset={} (rmii tx_en={} txd0={} txd1={} crs_dv={} rxd0={} rxd1={})",
        pins::ETH_MDC_GPIO,
        pins::ETH_MDIO_GPIO,
        pins::ETH_REF_CLK_GPIO,
        pins::ETH_PHY_RESET_GPIO,
        pins::ETH_TX_EN_GPIO,
        pins::ETH_TXD0_GPIO,
        pins::ETH_TXD1_GPIO,
        pins::ETH_CRS_DV_GPIO,
        pins::ETH_RXD0_GPIO,
        pins::ETH_RXD1_GPIO,
    );

    Ok(EthLink { netif, handle, mac })
}
