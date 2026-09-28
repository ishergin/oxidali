use core::ffi::CStr;

use esp_idf_svc::hal::cpu::Core;
use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;

pub fn on_app_core<F>(name: &'static CStr, stack_bytes: usize, priority: u8, body: F)
where
    F: FnOnce() + Send + 'static,
{
    let config = ThreadSpawnConfiguration {
        name: Some(name),
        stack_size: stack_bytes,
        priority,
        pin_to_core: Some(Core::Core1),
        ..Default::default()
    };
    config.set().expect("thread spawn configuration");
    std::thread::Builder::new()
        .stack_size(stack_bytes)
        .spawn(body)
        .expect("thread spawn");
    ThreadSpawnConfiguration::default()
        .set()
        .expect("thread spawn configuration restore");
}
