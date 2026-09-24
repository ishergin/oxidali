use core::ffi::CStr;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

const OBSERVATION_STALE_MS: u32 = 120_000;

struct ObservationSlot {
    sequence: AtomicU32,
    instance: AtomicU32,
    free: AtomicU32,
    sampled_ms: AtomicU32,
}

impl ObservationSlot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU32::new(0),
            instance: AtomicU32::new(0),
            free: AtomicU32::new(0),
            sampled_ms: AtomicU32::new(0),
        }
    }
}

static HTTPD_OBSERVATION: ObservationSlot = ObservationSlot::new();
static MQTT_OBSERVATION: ObservationSlot = ObservationSlot::new();

#[derive(Debug, Clone, Copy)]
pub struct TaskObservation {
    pub name: &'static CStr,
    pub instance: u32,
    pub free: u32,
    pub age_ms: u32,
    pub fresh: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct KnownTask {
    pub handle: usize,
    pub name: &'static CStr,
    kernel: bool,
}

static TASKS: Mutex<Vec<KnownTask>> = Mutex::new(Vec::new());

fn table() -> std::sync::MutexGuard<'static, Vec<KnownTask>> {
    TASKS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn current_handle() -> usize {
    #[cfg(target_os = "espidf")]
    {
        // SAFETY: pure FFI query about the calling task.
        unsafe { esp_idf_svc::sys::xTaskGetCurrentTaskHandle() as usize }
    }
    #[cfg(not(target_os = "espidf"))]
    {
        0
    }
}

pub fn register_current(name: &'static CStr) {
    let handle = current_handle();
    if handle == 0 {
        return;
    }
    let mut t = table();
    t.retain(|k| k.handle != handle);
    t.push(KnownTask {
        handle,
        name,
        kernel: false,
    });
}

pub fn unregister_current() {
    let handle = current_handle();
    if handle == 0 {
        return;
    }
    table().retain(|k| k.handle != handle);
}

pub fn observe_current(name: &'static CStr) {
    #[cfg(target_os = "espidf")]
    {
        // SAFETY: this reads the calling task's own stack watermark.
        let free = unsafe { esp_idf_svc::sys::uxTaskGetStackHighWaterMark(core::ptr::null_mut()) };
        observe_current_with_free(name, free as u32);
    }
    #[cfg(not(target_os = "espidf"))]
    let _ = name;
}

pub fn observe_current_with_free(name: &'static CStr, free: u32) {
    let Some(slot) = observation_slot(name) else {
        return;
    };
    #[cfg(target_os = "espidf")]
    {
        if !current_task_is(name) {
            return;
        }
        // SAFETY: both calls query the current task and the monotonic clock.
        let instance = unsafe { esp_idf_svc::sys::xTaskGetCurrentTaskHandle() as usize as u32 };
        publish_observation(slot, free, instance, monotonic_ms());
    }
    #[cfg(not(target_os = "espidf"))]
    let _ = (slot, free);
}

#[cfg(target_os = "espidf")]
fn current_task_is(name: &CStr) -> bool {
    // SAFETY: a null handle names the calling task, whose own name field lives while it runs.
    let actual = unsafe { esp_idf_svc::sys::pcTaskGetName(core::ptr::null_mut()) };
    if actual.is_null() {
        return false;
    }
    // SAFETY: FreeRTOS keeps the name NUL-terminated within its own buffer.
    unsafe { CStr::from_ptr(actual) }.to_bytes() == name.to_bytes()
}

#[cfg_attr(not(target_os = "espidf"), allow(dead_code, reason = "only the ESP build calls this"))]
fn publish_observation(slot: &ObservationSlot, free: u32, instance: u32, sampled_ms: u32) {
    let sequence = slot.sequence.load(Ordering::Relaxed).wrapping_add(1) | 1;
    slot.sequence.store(sequence, Ordering::Relaxed);
    core::sync::atomic::fence(Ordering::Release);
    slot.instance.store(instance, Ordering::Relaxed);
    slot.free.store(free, Ordering::Relaxed);
    slot.sampled_ms.store(sampled_ms, Ordering::Relaxed);
    slot.sequence
        .store(sequence.wrapping_add(1), Ordering::Release);
}

fn observation_slot(name: &CStr) -> Option<&'static ObservationSlot> {
    match name.to_bytes() {
        b"httpd" => Some(&HTTPD_OBSERVATION),
        b"mqtt_task" => Some(&MQTT_OBSERVATION),
        _ => None,
    }
}

pub fn for_each_observed(mut f: impl FnMut(Option<TaskObservation>, &'static CStr)) {
    let now_ms = monotonic_ms();
    for name in [c"httpd", c"mqtt_task"] {
        let sample = observation_slot(name).and_then(|slot| read_observation(slot, name, now_ms));
        f(sample, name);
    }
}

fn read_observation(
    slot: &ObservationSlot,
    name: &'static CStr,
    now_ms: u32,
) -> Option<TaskObservation> {
    let before = slot.sequence.load(Ordering::Acquire);
    if before == 0 || before & 1 != 0 {
        return None;
    }
    let instance = slot.instance.load(Ordering::Relaxed);
    let free = slot.free.load(Ordering::Relaxed);
    let age_ms = now_ms.wrapping_sub(slot.sampled_ms.load(Ordering::Relaxed));
    core::sync::atomic::fence(Ordering::Acquire);
    let after = slot.sequence.load(Ordering::Relaxed);
    (before == after).then_some(TaskObservation {
        name,
        instance,
        free,
        age_ms,
        fresh: age_ms <= OBSERVATION_STALE_MS,
    })
}

fn monotonic_ms() -> u32 {
    #[cfg(target_os = "espidf")]
    return (unsafe { esp_idf_svc::sys::esp_timer_get_time() as u64 } / 1000) as u32;
    #[cfg(not(target_os = "espidf"))]
    return 0;
}

pub fn refresh_kernel_tasks(names: &[&'static CStr]) {
    refresh_kernel_tasks_with(names, |name| {
        #[cfg(target_os = "espidf")]
        {
            // SAFETY: a NUL-terminated name; the kernel returns NULL for a task it does not have.
            unsafe { esp_idf_svc::sys::xTaskGetHandle(name.as_ptr()) as usize }
        }
        #[cfg(not(target_os = "espidf"))]
        {
            let _ = name;
            0
        }
    });
}

fn refresh_kernel_tasks_with(names: &[&'static CStr], resolve: impl Fn(&'static CStr) -> usize) {
    let resolved: Vec<KnownTask> = names
        .iter()
        .filter_map(|&name| {
            let handle = resolve(name);
            (handle != 0).then_some(KnownTask {
                handle,
                name,
                kernel: true,
            })
        })
        .collect();
    let mut t = table();
    t.retain(|k| !k.kernel);
    t.extend(resolved);
}

pub fn name_of(handle: usize) -> Option<&'static CStr> {
    table().iter().find(|k| k.handle == handle).map(|k| k.name)
}

pub fn for_each_with_stack_free(mut f: impl FnMut(&KnownTask, u32)) {
    let t = table();
    for task in t.iter() {
        #[cfg(target_os = "espidf")]
        let free = {
            // SAFETY: the table is held, so our threads cannot exit; kernel handles were re-resolved at this census's start.
            let free_bytes = unsafe {
                esp_idf_svc::sys::uxTaskGetStackHighWaterMark(
                    task.handle as esp_idf_svc::sys::TaskHandle_t,
                )
            };
            free_bytes as u32
        };
        #[cfg(not(target_os = "espidf"))]
        let free = 0u32;
        f(task, free);
    }
}

pub fn snapshot_stack_free(out: &mut Vec<(&'static CStr, u32)>) {
    out.clear();
    out.reserve(SNAPSHOT_RESERVE);
    for_each_with_stack_free(|task, free| out.push((task.name, free)));
}

const SNAPSHOT_RESERVE: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_published_observation_is_never_read_half_written() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        static SLOT: ObservationSlot = ObservationSlot::new();
        let stop = Arc::new(AtomicBool::new(false));
        let writer_stop = Arc::clone(&stop);
        let writer = std::thread::spawn(move || {
            let mut free = 1u32;
            while !writer_stop.load(Ordering::Relaxed) {
                publish_observation(&SLOT, free, free.wrapping_mul(7), free.wrapping_add(1));
                free = free.wrapping_add(1).max(1);
            }
        });

        let mut accepted = 0u32;
        for _ in 0..200_000 {
            if let Some(sample) = read_observation(&SLOT, c"httpd", 0) {
                assert_eq!(
                    sample.instance,
                    sample.free.wrapping_mul(7),
                    "instance came from a different write than free"
                );
                assert_eq!(
                    0u32.wrapping_sub(sample.age_ms),
                    sample.free.wrapping_add(1),
                    "the timestamp came from a different write than free"
                );
                accepted += 1;
            }
        }
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        assert!(accepted > 0, "the reader never observed a complete sample");
    }

    #[test]
    fn a_write_in_flight_and_an_untouched_slot_both_read_as_nothing() {
        static NEVER: ObservationSlot = ObservationSlot::new();
        assert!(read_observation(&NEVER, c"httpd", 0).is_none());

        static MIDWAY: ObservationSlot = ObservationSlot::new();
        MIDWAY.sequence.store(3, Ordering::Release);
        assert!(read_observation(&MIDWAY, c"httpd", 0).is_none());
        publish_observation(&MIDWAY, 512, 0xABCD, 4);
        let sample = read_observation(&MIDWAY, c"httpd", 10).expect("a finished write is readable");
        assert_eq!((sample.free, sample.instance, sample.age_ms), (512, 0xABCD, 6));
        assert!(sample.fresh);
    }

    #[test]
    fn a_sample_older_than_the_window_is_stale_rather_than_absent() {
        static AGED: ObservationSlot = ObservationSlot::new();
        publish_observation(&AGED, 900, 1, 0);
        let sample = read_observation(&AGED, c"mqtt_task", OBSERVATION_STALE_MS + 1)
            .expect("an old sample is still a sample");
        assert!(!sample.fresh);
        assert_eq!(sample.free, 900);
    }

    use super::*;

    #[test]
    fn a_refresh_replaces_kernel_handles_and_forgets_the_vanished() {
        let names: &[&'static CStr] = &[c"httpd", c"mqtt_task"];
        {
            let mut t = table();
            t.clear();
        }

        refresh_kernel_tasks_with(names, |n| if n == c"httpd" { 0x1000 } else { 0x2000 });
        let first: Vec<usize> = table().iter().map(|k| k.handle).collect();
        assert_eq!(first, vec![0x1000, 0x2000]);

        refresh_kernel_tasks_with(names, |n| if n == c"httpd" { 0 } else { 0x3000 });
        let second: Vec<(usize, &CStr)> =
            table().iter().map(|k| (k.handle, k.name)).collect();
        assert_eq!(
            second,
            vec![(0x3000, c"mqtt_task")],
            "the stale handle must be gone, not kept beside the new one"
        );
    }

    #[test]
    fn a_kernel_refresh_leaves_our_own_threads_alone() {
        {
            let mut t = table();
            t.clear();
            t.push(KnownTask {
                handle: 0xAAAA,
                name: c"dali_worker",
                kernel: false,
            });
        }
        refresh_kernel_tasks_with(&[c"httpd"], |_| 0x1000);
        let rows: Vec<(usize, bool)> = table().iter().map(|k| (k.handle, k.kernel)).collect();
        assert_eq!(rows, vec![(0xAAAA, false), (0x1000, true)]);
    }
}
