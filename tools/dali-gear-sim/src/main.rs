mod answer;
mod boot_slot;
mod console;
mod logsink;
mod phy;
mod spawn;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_gear_model::GearFleet;
use esp_idf_svc::hal::peripherals::Peripherals;

use answer::{AnswerLoop, LoopStats};
use console::Console;

const BUILD: &str = env!("DALI_GEAR_SIM_BUILD");

const FLEET_SEED: u32 = 0x0DA1_1000;

const GEAR_TASK_STACK: usize = 8192;
const GEAR_TASK_PRIORITY: u8 = 10;
const CONSOLE_TASK_STACK: usize = 8192;
const CONSOLE_TASK_PRIORITY: u8 = 5;

const FIRST_TICKS_WAIT: Duration = Duration::from_millis(50);

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    logsink::start();
    let slot = boot_slot::read();
    note!("dali-gear-sim build={BUILD} (esp32p4): no reserve and no fleet until the console sets them");
    let phy = start_phy();
    let fleet = Arc::new(Mutex::new(GearFleet::new(Vec::new(), 0, FLEET_SEED)));
    let stats = Arc::new(Mutex::new(LoopStats::default()));
    let (gear_fleet, gear_stats) = (Arc::clone(&fleet), Arc::clone(&stats));
    spawn::on_app_core(c"dali-gear", GEAR_TASK_STACK, GEAR_TASK_PRIORITY, move || {
        AnswerLoop::new(phy, gear_fleet, gear_stats).run()
    });
    let console = Console::new(phy, fleet, stats, FLEET_SEED, BUILD, slot);
    spawn::on_app_core(c"gear-console", CONSOLE_TASK_STACK, CONSOLE_TASK_PRIORITY, move || {
        console.run()
    });
}

fn start_phy() -> &'static phy::GearPhy {
    let peripherals = Peripherals::take().expect("peripherals");
    let phy = phy::GearPhy::start(peripherals).expect("DALI PHY");
    let (edges, level) = phy.probe();
    note!("pinscan rx=gpio{} level={} edges={edges}", phy::DALI_RX_GPIO, u8::from(level));
    if edges == 0 {
        note!("NOTE no bus activity seen — quiet segment, or the RX pin is wrong");
    }
    // sleep-ok: one boot-time wait for the first timer ticks before asking which core took the interrupt.
    std::thread::sleep(FIRST_TICKS_WAIT);
    match phy::isr_core() {
        Some(core) => note!(
            "phy up: tx=gpio{} rx=gpio{}, isr on core {core} at priority {}",
            phy::DALI_TX_GPIO,
            phy::DALI_RX_GPIO,
            phy::PHY_INTERRUPT_PRIORITY
        ),
        None => note!("WARNING phy timer has not ticked — the emulator is deaf and mute"),
    }
    phy
}
