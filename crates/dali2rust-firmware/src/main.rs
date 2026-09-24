#![allow(unexpected_cfgs, reason = "ESP-IDF target cfg is external")]

#[cfg(target_os = "espidf")]
use embassy_executor as _;

#[cfg(target_os = "espidf")]
#[global_allocator]
static ALLOCATOR: dali2rust_bsp::rust_heap::CountingAllocator =
    dali2rust_bsp::rust_heap::CountingAllocator;

#[cfg(target_os = "espidf")]
fn main() {
    dali2rust_firmware::composition::run();
}

#[cfg(not(target_os = "espidf"))]
fn main() {
    eprintln!(
        "This binary targets ESP-IDF. Build with: cargo build --target riscv32imafc-esp-espidf"
    );
    std::process::exit(1);
}
