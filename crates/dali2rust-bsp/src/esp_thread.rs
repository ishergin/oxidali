use core::ffi::CStr;
use core::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackHome {
    Internal,
    External,
    ExternalOnXip,
}

static EXTERNAL_ON_XIP: AtomicBool = AtomicBool::new(true);

pub fn keep_stacks_internal() {
    EXTERNAL_ON_XIP.store(false, Ordering::Relaxed);
}

pub fn external_stacks_on_xip() -> bool {
    cfg!(esp_idf_spiram_xip_from_psram) && EXTERNAL_ON_XIP.load(Ordering::Relaxed)
}

impl StackHome {
    fn resolve(self) -> Self {
        match self {
            Self::ExternalOnXip if external_stacks_on_xip() => Self::External,
            Self::ExternalOnXip => Self::Internal,
            placed => placed,
        }
    }
}

#[cfg(target_os = "espidf")]
pub fn current_stack_is_external() -> bool {
    let marker = 0u8;
    crate::rust_heap::placement_of(
        core::ptr::addr_of!(marker) as usize,
        esp_idf_svc::sys::SOC_EXTRAM_LOW as usize,
        esp_idf_svc::sys::SOC_EXTRAM_HIGH as usize,
    ) == crate::rust_heap::Placement::Psram
}

pub fn spawn_named_stack_in<F>(
    name: &'static CStr,
    stack_bytes: usize,
    home: StackHome,
    f: F,
) -> std::thread::JoinHandle<()>
where
    F: FnOnce() + Send + 'static,
{
    try_spawn_named_stack_in(name, stack_bytes, home, None, f)
        .unwrap_or_else(|e| panic!("spawn thread {}: {e:?}", name.to_string_lossy()))
}

pub fn try_spawn_named_stack_in<F>(
    name: &'static CStr,
    stack_bytes: usize,
    home: StackHome,
    core: Option<u8>,
    f: F,
) -> std::io::Result<std::thread::JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    with_named_builder_on(name, stack_bytes, home, core, |b| b.spawn(registered(name, f)))
}

pub fn run_on_named_stack_in<R, F>(
    name: &'static CStr,
    stack_bytes: usize,
    home: StackHome,
    f: F,
) -> std::io::Result<R>
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    run_scoped(name, stack_bytes, home, f)
}

pub fn spawn_named_stack<F>(
    name: &'static CStr,
    stack_bytes: usize,
    f: F,
) -> std::thread::JoinHandle<()>
where
    F: FnOnce() + Send + 'static,
{
    try_spawn_named_stack(name, stack_bytes, f)
        .unwrap_or_else(|e| panic!("spawn thread {}: {e:?}", name.to_string_lossy()))
}

pub fn try_spawn_named_stack<F>(
    name: &'static CStr,
    stack_bytes: usize,
    f: F,
) -> std::io::Result<std::thread::JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    with_named_builder(name, stack_bytes, StackHome::Internal, |b| {
        b.spawn(registered(name, f))
    })
}

pub fn try_spawn_named_stack_on_core<F>(
    name: &'static CStr,
    stack_bytes: usize,
    core: u8,
    f: F,
) -> std::io::Result<std::thread::JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    with_named_builder_on(name, stack_bytes, StackHome::Internal, Some(core), |b| {
        b.spawn(registered(name, f))
    })
}

pub fn try_spawn_named_external_stack_on_core<F>(
    name: &'static CStr,
    stack_bytes: usize,
    core: u8,
    f: F,
) -> std::io::Result<std::thread::JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    with_named_builder_on(name, stack_bytes, StackHome::External, Some(core), |b| {
        b.spawn(registered(name, f))
    })
}

pub fn run_on_named_stack<R, F>(name: &'static CStr, stack_bytes: usize, f: F) -> std::io::Result<R>
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    run_scoped(name, stack_bytes, StackHome::Internal, f)
}

pub fn run_on_external_stack<R, F>(
    name: &'static CStr,
    stack_bytes: usize,
    f: F,
) -> std::io::Result<R>
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    run_scoped(name, stack_bytes, StackHome::External, f)
}

fn registered<R, F>(name: &'static CStr, f: F) -> impl FnOnce() -> R + Send
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    move || {
        crate::task_registry::register_current(name);
        struct RegistrationGuard;
        impl Drop for RegistrationGuard {
            fn drop(&mut self) {
                crate::task_registry::unregister_current();
            }
        }
        let _guard = RegistrationGuard;
        f()
    }
}

fn run_scoped<R, F>(
    name: &'static CStr,
    stack_bytes: usize,
    home: StackHome,
    f: F,
) -> std::io::Result<R>
where
    F: FnOnce() -> R + Send,
    R: Send,
{
    #[cfg(target_os = "espidf")]
    {
        std::thread::scope(|scope| {
            let handle = with_named_builder(name, stack_bytes, home, |b| {
                b.spawn_scoped(scope, registered(name, f))
            })?;
            Ok(handle
                .join()
                .unwrap_or_else(|e| std::panic::resume_unwind(e)))
        })
    }
    #[cfg(not(target_os = "espidf"))]
    {
        let _ = (name, stack_bytes, home);
        Ok(f())
    }
}

fn with_named_builder<T>(
    name: &'static CStr,
    stack_bytes: usize,
    home: StackHome,
    spawn: impl FnOnce(std::thread::Builder) -> std::io::Result<T>,
) -> std::io::Result<T> {
    with_named_builder_on(name, stack_bytes, home, None, spawn)
}

fn with_named_builder_on<T>(
    name: &'static CStr,
    stack_bytes: usize,
    home: StackHome,
    core: Option<u8>,
    spawn: impl FnOnce(std::thread::Builder) -> std::io::Result<T>,
) -> std::io::Result<T> {
    let label = name.to_string_lossy();
    let mut b = std::thread::Builder::new().name(label.to_string());
    if stack_bytes > 0 {
        b = b.stack_size(stack_bytes);
    }
    #[cfg(target_os = "espidf")]
    {
        use esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration;

        let previous = ThreadSpawnConfiguration::get();
        if let Err(e) = spawn_configuration(name, home, core).set() {
            log::warn!("thread-name config for {label} failed: {e}");
        }
        let spawned = spawn(b);
        if let Err(e) = &spawned {
            log::error!("spawn thread {label} failed: {e:?}");
        }
        if let Err(e) = previous.unwrap_or_default().set() {
            log::warn!("thread-name config restore after {label} failed: {e}");
        }
        spawned
    }
    #[cfg(not(target_os = "espidf"))]
    {
        let _ = (home.resolve(), core);
        spawn(b)
    }
}

#[cfg(target_os = "espidf")]
fn spawn_configuration(
    name: &'static CStr,
    home: StackHome,
    core: Option<u8>,
) -> esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration {
    use esp_idf_svc::hal::cpu::Core;
    use esp_idf_svc::hal::task::thread::{MallocCap, ThreadSpawnConfiguration};

    let mut config = ThreadSpawnConfiguration {
        name: Some(name),
        ..Default::default()
    };
    if home.resolve() == StackHome::External {
        config.stack_alloc_caps = MallocCap::Spiram | MallocCap::Cap8bit;
    }
    config.pin_to_core = core.map(|c| if c == 0 { Core::Core0 } else { Core::Core1 });
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_build_is_no_xip_image_so_every_conditional_stack_stays_internal() {
        assert!(!external_stacks_on_xip());
        assert_eq!(StackHome::ExternalOnXip.resolve(), StackHome::Internal);
        assert_eq!(StackHome::External.resolve(), StackHome::External);
        assert_eq!(StackHome::Internal.resolve(), StackHome::Internal);
    }
}
