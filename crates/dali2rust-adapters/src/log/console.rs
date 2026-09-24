use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::OnceLock;
use std::time::Duration;

use dali2rust_platform::logs::{ConsoleLogCounters, LogLevel};
use esp_idf_svc::sys::{
    esp_rom_printf, uart_disable_rx_intr, uart_driver_install, uart_is_driver_installed,
    uart_write_bytes, CONFIG_ESP_CONSOLE_UART_NUM, ESP_OK,
};

use super::console_queue::{ConsoleQueue, FillResult, PopOutcome, SLOT_BYTES};

const CONSOLE: u32 = CONFIG_ESP_CONSOLE_UART_NUM;
const RX_RING_BYTES: i32 = 256;
const TX_RING_BYTES: i32 = 2048;
const WRITER_STACK_BYTES: usize = 4 * 1024;
const WRITER_CORE: u8 = 1;
const WRITER_BURST: usize = 8;
const WRITER_IDLE_WAIT_MS: u64 = 10;
const WRITER_BUSY_RETRIES: usize = 4;

static QUEUE: OnceLock<ConsoleQueue> = OnceLock::new();
static WAKE: OnceLock<SyncSender<()>> = OnceLock::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);
static COUNTERS: ConsoleLogCounters = ConsoleLogCounters::new();

pub fn install() -> bool {
    if INSTALLED.load(Ordering::Acquire) {
        return true;
    }
    let Some(queue) = ConsoleQueue::try_new(&COUNTERS) else {
        COUNTERS.unavailable.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel::<()>(1);
    if install_uart_driver() != ESP_OK || QUEUE.set(queue).is_err() || WAKE.set(wake_tx).is_err() {
        COUNTERS.unavailable.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let spawned = dali2rust_bsp::esp_thread::try_spawn_named_external_stack_on_core(
        c"log-uart",
        WRITER_STACK_BYTES,
        WRITER_CORE,
        move || writer_loop(wake_rx),
    );
    if spawned.is_err() {
        COUNTERS.unavailable.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    INSTALLED.store(true, Ordering::Release);
    true
}

fn install_uart_driver() -> i32 {
    // SAFETY: the ROM console configured the pins; no queue handle is requested; console logging is ours.
    let result = unsafe {
        if uart_is_driver_installed(CONSOLE) {
            ESP_OK
        } else {
            uart_driver_install(
                CONSOLE,
                RX_RING_BYTES,
                TX_RING_BYTES,
                0,
                core::ptr::null_mut(),
                0,
            )
        }
    };
    if result == ESP_OK {
        // SAFETY: the driver is installed and the console is write-only.
        unsafe { uart_disable_rx_intr(CONSOLE) };
    }
    result
}

fn writer_loop(wake: Receiver<()>) {
    let mut line = [0u8; SLOT_BYTES];
    loop {
        drain_queue(&mut line);
        let _ = wake.recv_timeout(Duration::from_millis(WRITER_IDLE_WAIT_MS));
    }
}

fn drain_queue(line: &mut [u8; SLOT_BYTES]) {
    let Some(queue) = QUEUE.get() else { return };
    let mut burst = 0usize;
    let mut busy = 0usize;
    loop {
        match queue.try_pop(line) {
            PopOutcome::Line(len) => {
                write_line(&line[..len]);
                busy = 0;
                burst += 1;
            }
            PopOutcome::Empty => return,
            PopOutcome::Busy if busy < WRITER_BUSY_RETRIES => {
                busy += 1;
                // busy-wait-ok: the gate is held for one `fill`; yielding hands its holder the core back, bounded above.
                std::thread::yield_now();
                continue;
            }
            PopOutcome::Busy => return,
        }
        if burst == WRITER_BURST {
            burst = 0;
            std::thread::yield_now();
        }
    }
}

fn write_line(bytes: &[u8]) {
    // SAFETY: this task is the only UART writer after install; the queue gate is released before this call.
    let written = unsafe { uart_write_bytes(CONSOLE, bytes.as_ptr().cast(), bytes.len()) };
    if written < 0 || written as usize != bytes.len() {
        COUNTERS.uart_errors.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn try_write(level: LogLevel, fill: impl FnOnce(&mut [u8]) -> FillResult) -> bool {
    if !INSTALLED.load(Ordering::Acquire) {
        COUNTERS.unavailable.fetch_add(1, Ordering::Relaxed);
        COUNTERS.dropped.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let queued = QUEUE.get().is_some_and(|queue| queue.try_push(level, fill));
    if queued {
        if let Some(wake) = WAKE.get() {
            let _ = wake.try_send(());
        }
    }
    queued
}

#[cfg(target_os = "espidf")]
pub fn drain_to_rom_console() {
    const CHUNK: usize = 48;
    let Some(queue) = QUEUE.get() else {
        return;
    };
    for _ in 0..super::console_queue::slot_count() {
        if !queue.pop_in_place(|line| {
            let mut buffer = [0u8; CHUNK + 1];
            for piece in line.chunks(CHUNK) {
                buffer[..piece.len()].copy_from_slice(piece);
                buffer[piece.len()] = 0;
                // SAFETY: NUL-terminated within the buffer, and `%s` consumes it before this frame returns.
                unsafe { esp_rom_printf(c"%s".as_ptr(), buffer.as_ptr()) };
            }
        }) {
            return;
        }
    }
}

pub fn dropped() -> u32 {
    COUNTERS.dropped.load(Ordering::Relaxed)
}

pub fn busy() -> u32 {
    COUNTERS.busy.load(Ordering::Relaxed)
}

pub fn truncated() -> u32 {
    COUNTERS.truncated.load(Ordering::Relaxed)
}

pub fn unavailable() -> u32 {
    COUNTERS.unavailable.load(Ordering::Relaxed)
}

pub fn uart_errors() -> u32 {
    COUNTERS.uart_errors.load(Ordering::Relaxed)
}

pub fn counters() -> &'static ConsoleLogCounters {
    &COUNTERS
}

pub use super::console_queue::FillResult as ConsoleFillResult;
