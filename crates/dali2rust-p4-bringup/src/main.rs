#![allow(unexpected_cfgs, reason = "ESP-IDF target cfg is external")]

#[cfg(target_os = "espidf")]
use embassy_executor as _;

#[cfg(target_os = "espidf")]
mod dali_tx;

#[cfg(target_os = "espidf")]
mod wiring;

#[cfg(target_os = "espidf")]
mod imp {
    use std::sync::Arc;
    use std::time::Duration;

    use esp_idf_svc::hal::peripherals::Peripherals;

    use crate::wiring::scan_i2c;
    use crate::dali_tx;

    use dali2rust_platform::net::{LinkStats, NetworkLink};

    use esp_idf_svc::http::server::{Configuration, EspHttpServer};
    use esp_idf_svc::io::{EspIOError, Write};

    const HTTP_STACK_BYTES: usize = 8 * 1024;
    const HEARTBEAT_PERIOD: Duration = Duration::from_secs(5);

    fn stats_json(link: &dyn NetworkLink) -> String {
        let s = link.stats();
        let st = link.status();
        let ip = st
            .ipv4
            .map(|o| format!("\"{}.{}.{}.{}\"", o[0], o[1], o[2], o[3]))
            .unwrap_or_else(|| "null".into());
        format!(
            concat!(
                "{{\"link\":{{\"kind\":\"{}\",\"up\":{},\"speed_mbps\":{},",
                "\"full_duplex\":{},\"ipv4\":{},\"up_events\":{}}},",
                "\"counters\":{{\"rx_packets\":{},\"tx_packets\":{},",
                "\"rx_bytes\":{},\"tx_bytes\":{},\"rx_dropped\":{},",
                "\"tx_dropped\":{}}}}}"
            ),
            link.kind(),
            st.up,
            st.speed_mbps,
            st.full_duplex,
            ip,
            s.link_up_events,
            s.rx_packets,
            s.tx_packets,
            s.rx_bytes,
            s.tx_bytes,
            s.rx_dropped,
            s.tx_dropped,
        )
    }

    fn serve(link: Arc<dyn NetworkLink>) -> Result<EspHttpServer<'static>, EspIOError> {
        let mut conf = Configuration::default();
        conf.stack_size = HTTP_STACK_BYTES;
        let mut server = EspHttpServer::new(&conf)?;
        server.fn_handler::<EspIOError, _>("/", esp_idf_svc::http::Method::Get, move |req| {
            let body = stats_json(link.as_ref());
            let mut w = req.into_response(200, None, &[("Content-Type", "application/json")])?;
            w.write_all(body.as_bytes())?;
            Ok(())
        })?;
        Ok(server)
    }

    fn heartbeat_tick(link: &dyn NetworkLink, previous: LinkStats, seconds: u32) -> LinkStats {
        let now = link.stats();
        let d = now.since(previous);
        let st = link.status();
        log::info!(
            "eth: up={} {}Mb/{} ip={:?} | +{}pkt/+{}B rx  +{}pkt/+{}B tx  (per {}s)",
            st.up,
            st.speed_mbps,
            if st.full_duplex { "FD" } else { "HD" },
            st.ipv4,
            d.rx_packets,
            d.rx_bytes,
            d.tx_packets,
            d.tx_bytes,
            seconds,
        );
        if d.has_loss() {
            log::warn!(
                "eth LOSS: rx_dropped=+{} tx_dropped=+{} rx_ring_overruns=+{} rx_fifo_overflows=+{} \
                 (totals rx_drop={} tx_drop={})",
                d.rx_dropped,
                d.tx_dropped,
                d.rx_ring_overruns,
                d.rx_fifo_overflows,
                now.rx_dropped,
                now.tx_dropped,
            );
        }
        now
    }

    pub fn run() -> ! {
        esp_idf_svc::sys::link_patches();
        esp_idf_svc::log::EspLogger::initialize_default();
        log::info!("p4-bringup: ESP32-P4-ETH, Ethernet only (ADR-008)");

        let link: Arc<dyn NetworkLink> = match dali2rust_bsp::esp32p4::eth::start() {
            Ok(l) => Arc::new(l),
            Err(e) => {
                log::error!("eth: bring-up FAILED: {e:?}");
                loop {
                    // sleep-ok: nothing to do but stay alive for the log.
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
        };

        let _server = match serve(Arc::clone(&link)) {
            Ok(s) => Some(s),
            Err(e) => {
                log::error!("http: {e:?} — continuing, the heartbeat still reports");
                None
            }
        };

        let peripherals = Peripherals::take().expect("peripherals");
        scan_i2c(
            peripherals.i2c0,
            peripherals.pins.gpio33,
            peripherals.pins.gpio32,
        );
        let _tx_probe = dali_tx::start(peripherals.pins.gpio14, peripherals.pins.gpio17);

        heartbeat_forever(link.as_ref());
    }

    fn heartbeat_forever(link: &dyn NetworkLink) -> ! {
        let mut previous = link.stats();
        loop {
            // sleep-ok: fixed heartbeat cadence, far above the 1 ms floor.
            std::thread::sleep(HEARTBEAT_PERIOD);
            previous = heartbeat_tick(link, previous, HEARTBEAT_PERIOD.as_secs() as u32);
        }
    }
}

#[cfg(target_os = "espidf")]
fn main() -> ! {
    imp::run()
}

#[cfg(not(target_os = "espidf"))]
fn main() {
    println!("dali2rust-p4-bringup targets ESP32-P4 only; build with `just p4-flash`.");
}
