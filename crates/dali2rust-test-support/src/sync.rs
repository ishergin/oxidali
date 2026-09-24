use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

pub const WORKER_SETTLE: Duration = Duration::from_millis(120);
pub const SHORT_POLL: Duration = Duration::from_millis(10);

pub fn recv_with_deadline<T>(rx: &Receiver<T>, timeout: Duration) -> T {
    rx.recv_timeout(timeout)
        .unwrap_or_else(|err| panic!("timed out after {timeout:?}: {err}"))
}

pub fn wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return;
        }
        if Instant::now() >= deadline {
            break;
        }
        // sleep-ok: SHORT_POLL heartbeat inside the deadline-poll helper itself
        thread::sleep(SHORT_POLL);
    }
    panic!("condition not satisfied within {timeout:?}");
}

pub fn try_wait_until(mut condition: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        // sleep-ok: SHORT_POLL heartbeat inside the deadline-poll helper itself
        thread::sleep(SHORT_POLL);
    }
    condition()
}

pub fn remains_false_for(mut condition: impl FnMut() -> bool, window: Duration) -> bool {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        if condition() {
            return false;
        }
        // sleep-ok: SHORT_POLL heartbeat inside the deadline-poll helper itself
        thread::sleep(SHORT_POLL);
    }
    true
}

pub fn wait_for_tcp_ready(port: u16, timeout: Duration) {
    wait_until(
        || {
            TcpStream::connect(("127.0.0.1", port))
                .map(|stream| {
                    let _ = stream.shutdown(Shutdown::Both);
                    true
                })
                .unwrap_or(false)
        },
        timeout,
    );
}

pub fn await_counter_u32(
    counter: &AtomicU32,
    predicate: impl Fn(u32) -> bool,
    timeout: Duration,
) {
    wait_until(
        || predicate(counter.load(Ordering::Relaxed)),
        timeout,
    );
}

pub fn await_counter_u64(
    counter: &AtomicU64,
    predicate: impl Fn(u64) -> bool,
    timeout: Duration,
) {
    wait_until(
        || predicate(counter.load(Ordering::Relaxed)),
        timeout,
    );
}
