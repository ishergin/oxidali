#[cfg(target_os = "espidf")]
pub mod console;
mod console_queue;
#[cfg(target_os = "espidf")]
pub mod esp_idf;

mod line;

#[cfg(not(target_os = "espidf"))]
static HOST_CONSOLE_COUNTERS: dali2rust_platform::logs::ConsoleLogCounters =
    dali2rust_platform::logs::ConsoleLogCounters::new();

pub fn console_counters() -> &'static dali2rust_platform::logs::ConsoleLogCounters {
    #[cfg(target_os = "espidf")]
    return console::counters();
    #[cfg(not(target_os = "espidf"))]
    return &HOST_CONSOLE_COUNTERS;
}
