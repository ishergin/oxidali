use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};


use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_adapters::http::host::{run_blocking, HostServer};
use dali2rust_adapters::{
    build_router_with_bus_and_transport, HardwareDisplay, StackOptions, StaticAsset,
};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";
const BIND_ADDR_ENV: &str = "DALI2RUST_DEV_SERVER_ADDR";
const PORT_ENV: &str = "PORT";
const DEFAULT_WEB_DIST_GZ: &str = "web/app/dist-gz";
const WEB_DIST_ENV: &str = "DALI2RUST_WEB_DIST_GZ";
const FLEET_ENV: &str = "DALI2RUST_DEV_SERVER_FLEET";
const BENCH_FLEET: &str = "bench";
const BENCH_FLEET_LAYOUT: (u8, u8, u8, u8) = (0, 34, 20, 10);
const BENCH_FLEET_SEED: u32 = 0x0DA1_1000;
const DEV_SERVER_VERSION: &str = "dev-host";

fn asset_route(rel: &str) -> Option<(&'static str, &'static str)> {
    match rel {
        "index.html.gz" => Some(("/", "text/html")),
        "assets/app.js.gz" => Some(("/assets/app.js", "application/javascript")),
        "assets/app.css.gz" => Some(("/assets/app.css", "text/css")),
        "favicon.svg.gz" => Some(("/favicon.svg", "image/svg+xml")),
        "dali-products.json.gz" => Some(("/dali-products.json", "application/json")),
        _ => None,
    }
}

fn load_web_assets(dist_gz: &Path) -> &'static [StaticAsset] {
    let mut assets = Vec::new();
    for rel in [
        "index.html.gz",
        "assets/app.js.gz",
        "assets/app.css.gz",
        "favicon.svg.gz",
        "dali-products.json.gz",
    ] {
        let Some((route_path, content_type)) = asset_route(rel) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(dist_gz.join(rel)) else {
            continue;
        };
        // SAFETY: one leak per asset at start-up; the composition API takes 'static bytes.
        let gzip_body: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        assets.push(StaticAsset {
            route_path,
            content_type,
            gzip_body,
        });
    }
    // SAFETY: same one-time leak as above; the server owns these for its lifetime.
    Box::leak(assets.into_boxed_slice())
}

fn console_lines(transport: &Arc<Mutex<SimDaliTransport>>) {
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        match stdin.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((verb, args)) = words.split_first() else {
            continue;
        };
        match console_command(transport, verb, args) {
            Ok(message) => println!("  {message}"),
            Err(message) => println!("  ? {message}"),
        }
    }
}

const CONSOLE_HELP: &str = "press D I | release D I | foreign HEX16 | occupancy D I VALUE INFO | help";

fn console_command(
    transport: &Arc<Mutex<SimDaliTransport>>,
    verb: &str,
    args: &[&str],
) -> Result<String, String> {
    let mut sim = transport.lock().expect("sim transport lock");
    match verb {
        "press" | "release" => {
            let (device, instance) = two_numbers(args)?;
            let sent = sim.press_button(device, instance, verb == "press");
            Ok(format!("{verb} dev={device} inst={instance}: {sent} event frame(s)"))
        }
        "foreign" => {
            let raw = args.first().ok_or("foreign needs a 16-bit frame in hex")?;
            let frame = u16::from_str_radix(raw.trim_start_matches("0x"), 16)
                .map_err(|_| "not a 16-bit hex frame".to_string())?;
            let reached = sim.inject_foreign_frame(frame);
            Ok(format!("foreign 0x{frame:04X}: {}",
                       if reached { "delivered" } else { "nothing listening" }))
        }
        "occupancy" => {
            let (device, instance) = two_numbers(args)?;
            let value = number(args.get(2), "VALUE")?;
            let info = number(args.get(3), "INFO")?;
            let sent = sim.occupancy_transition(
                device,
                instance,
                u8::try_from(value).map_err(|_| "VALUE is one byte".to_string())?,
                u16::try_from(info).map_err(|_| "INFO is ten bits".to_string())?,
            );
            Ok(format!("occupancy dev={device} inst={instance}: {sent} event frame(s)"))
        }
        "help" => Ok(CONSOLE_HELP.to_string()),
        other => Err(format!("unknown command {other:?} — {CONSOLE_HELP}")),
    }
}

fn two_numbers(args: &[&str]) -> Result<(usize, usize), String> {
    Ok((number(args.first(), "DEVICE")?, number(args.get(1), "INSTANCE")?))
}

fn number(arg: Option<&&str>, what: &str) -> Result<usize, String> {
    let raw = arg.ok_or_else(|| format!("missing {what} — {CONSOLE_HELP}"))?;
    let (radix, digits) = raw
        .strip_prefix("0x")
        .map_or((10, *raw), |hex| (16, hex));
    usize::from_str_radix(digits, radix).map_err(|_| format!("{what} is not a number: {raw:?}"))
}

fn main() {
    let addr = std::env::var(BIND_ADDR_ENV)
        .or_else(|_| std::env::var(PORT_ENV).map(|port| format!("127.0.0.1:{port}")))
        .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());
    let dist_gz =
        std::env::var(WEB_DIST_ENV).unwrap_or_else(|_| DEFAULT_WEB_DIST_GZ.to_string());
    let web_assets = load_web_assets(Path::new(&dist_gz));
    let bench_fleet = std::env::var(FLEET_ENV).is_ok_and(|value| value == BENCH_FLEET);
    let transport = Arc::new(Mutex::new(if bench_fleet {
        let (base, dt6, cct, rgb) = BENCH_FLEET_LAYOUT;
        SimDaliTransport::new(dali2rust_gear_model::bench_fleet(
            base,
            dt6,
            cct,
            rgb,
            BENCH_FLEET_SEED,
        ))
    } else {
        SimDaliTransport::demo_bus()
    }));
    let console_transport = Arc::clone(&transport);
    let (router, ws_hub, runtime) = build_router_with_bus_and_transport(
        DEV_SERVER_VERSION,
        transport,
        HardwareDisplay::none(),
        StackOptions {
            web_assets,
            ..StackOptions::default()
        },
    );

    let server = HostServer::bind(&addr).expect("bind dev server address");
    let stop = AtomicBool::new(false);
    println!("dali2rust host dev server listening on http://{addr}");
    if bench_fleet {
        println!("  simulated bus: 64 gear (34x DT6, 20x DT8 CCT, 10x DT8 RGB) on shorts 0-63");
    } else {
        println!("  simulated bus: 4x DT6 LED (short 0-3), DT8 CCT (4), DT8 RGB (5), DT8 RGBWAF 6ch (7),\n                  DT8 CCT + energy/diagnostics banks (8), DT8 + Part 251 luminaire (9),\n                  1 unaddressed");
        println!("  set {FLEET_ENV}={BENCH_FLEET} for a full 64-address segment");
    }
    if web_assets.is_empty() {
        println!("  web UI: not served (run scripts/build_web_ui.sh first)");
    } else {
        println!("  web UI: http://{addr}/ ({} assets)", web_assets.len());
    }
    println!("  try: curl http://{addr}/api/v1/health");
    println!("  console (stdin): {CONSOLE_HELP}");
    std::thread::spawn(move || console_lines(&console_transport));

    let result = run_blocking(&server, Arc::new(router), ws_hub, &stop);
    drop(runtime);
    result.expect("dev server loop");
}
