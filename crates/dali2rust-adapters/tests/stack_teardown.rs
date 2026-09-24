use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::MockDaliTransport;
use dali2rust_adapters::{build_http_test_stack, BusStackRuntime, DaliRuntimeConfig};
use dali2rust_bus::BusConfig;

fn build_and_drop() {
    let mock = Arc::new(Mutex::new(MockDaliTransport::new()));
    let (_router, _hub, runtime): (_, _, Box<BusStackRuntime>) = build_http_test_stack(
        "0.1.0-test",
        mock,
        BusConfig::default(),
        DaliRuntimeConfig::default(),
        None,
        &[],
        None,
    );
    drop(runtime);
}

#[cfg(target_os = "macos")]
fn live_threads() -> usize {
    #[repr(C)]
    #[derive(Default)]
    struct ProcTaskInfo {
        virtual_size: u64,
        resident_size: u64,
        total_user: u64,
        total_system: u64,
        threads_user: u64,
        threads_system: u64,
        policy: i32,
        faults: i32,
        pageins: i32,
        cow_faults: i32,
        messages_sent: i32,
        messages_received: i32,
        syscalls_mach: i32,
        syscalls_unix: i32,
        csw: i32,
        threadnum: i32,
        numrunning: i32,
        priority: i32,
    }
    unsafe extern "C" {
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut core::ffi::c_void,
            buffersize: i32,
        ) -> i32;
    }
    let mut info = ProcTaskInfo::default();
    let size = i32::try_from(size_of::<ProcTaskInfo>()).expect("task info size");
    // SAFETY: `info` is a live allocation of the flavour's layout, and the length passed is its own `size_of`.
    let written = unsafe {
        proc_pidinfo(
            std::process::id() as i32,
            4,
            0,
            std::ptr::from_mut(&mut info).cast(),
            size,
        )
    };
    assert_eq!(written, size, "proc_pidinfo(PROC_PIDTASKINFO) failed");
    usize::try_from(info.threadnum).expect("thread count")
}

#[cfg(target_os = "linux")]
fn live_threads() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .expect("/proc/self/status")
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .and_then(|n| n.trim().parse().ok())
        .expect("Threads: line")
}

fn settles_to(ceiling: usize) -> usize {
    let _ = dali2rust_test_support::try_wait_until(
        || live_threads() <= ceiling,
        std::time::Duration::from_secs(10),
    );
    live_threads()
}

const STACKS: usize = 20;

const SLACK: usize = 10;

#[cfg(unix)]
#[test]
fn twenty_composed_stacks_leave_no_threads_behind() {
    let baseline = settles_to(usize::MAX);
    for _ in 0..STACKS {
        build_and_drop();
    }
    let after = settles_to(baseline + SLACK);
    assert!(
        after <= baseline + SLACK,
        "{STACKS} stacks built and dropped left {after} threads against a baseline of \
         {baseline}: a worker is looping on a bare timer instead of waiting on its inbox"
    );
}
