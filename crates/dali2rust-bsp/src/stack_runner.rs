use core::ffi::CStr;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SendError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, PoisonError};

use crate::esp_thread::{try_spawn_named_stack_in, StackHome};

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug)]
pub enum RunError {
    Busy,
    Unstarted(std::io::Error),
    Unanswered,
}

impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Busy => f.write_str("no runner thread is free for the job"),
            Self::Unstarted(e) => write!(f, "the runner thread did not start: {e}"),
            Self::Unanswered => f.write_str("the job ended without an answer"),
        }
    }
}

const QUEUED_JOBS: usize = 1;
const ANSWER_SLOTS: usize = 1;

#[derive(Clone, Copy)]
enum Handover {
    WaitForRoom,
    OnlyIfRoom,
}

fn hand_over(queue: &SyncSender<Job>, job: Job, handover: Handover) -> Result<(), TrySendError<Job>> {
    match handover {
        Handover::WaitForRoom => queue.send(job).map_err(|SendError(job)| TrySendError::Disconnected(job)),
        Handover::OnlyIfRoom => queue.try_send(job),
    }
}

pub struct StackRunner {
    name: &'static CStr,
    stack_bytes: usize,
    home: StackHome,
    jobs: Mutex<Option<SyncSender<Job>>>,
}

impl StackRunner {
    pub const fn new(name: &'static CStr, stack_bytes: usize, home: StackHome) -> Self {
        Self {
            name,
            stack_bytes,
            home,
            jobs: Mutex::new(None),
        }
    }

    pub fn run<R, F>(&self, job: F) -> Result<R, RunError>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        let (answer, answered) = mpsc::sync_channel(ANSWER_SLOTS);
        self.submit(
            Box::new(move || {
                let _ = answer.send(job());
            }),
            Handover::WaitForRoom,
        )?;
        answered.recv().map_err(|_| RunError::Unanswered)
    }

    fn submit(&self, job: Job, handover: Handover) -> Result<(), RunError> {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        let job = match jobs.as_ref() {
            Some(queue) => match hand_over(queue, job, handover) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(_)) => return Err(RunError::Busy),
                Err(TrySendError::Disconnected(job)) => job,
            },
            None => job,
        };
        let queue = self.spawn().map_err(RunError::Unstarted)?;
        queue.try_send(job).map_err(|_| {
            RunError::Unstarted(std::io::Error::other("the runner thread ended before its first job"))
        })?;
        *jobs = Some(queue);
        Ok(())
    }

    fn spawn(&self) -> std::io::Result<SyncSender<Job>> {
        let (queue, jobs) = mpsc::sync_channel::<Job>(QUEUED_JOBS);
        try_spawn_named_stack_in(self.name, self.stack_bytes, self.home, None, move || serve(&jobs))?;
        Ok(queue)
    }
}

fn serve(jobs: &Receiver<Job>) {
    while let Ok(job) = jobs.recv() {
        let _ = std::panic::catch_unwind(AssertUnwindSafe(job));
    }
}

pub struct RunnerPool {
    slots: Box<[PoolSlot]>,
}

struct PoolSlot {
    runner: StackRunner,
    taken: Arc<AtomicBool>,
}

struct Release(Arc<AtomicBool>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl RunnerPool {
    pub fn new(name: &'static CStr, stack_bytes: usize, home: StackHome, runners: usize) -> Self {
        let slots = (0..runners)
            .map(|_| PoolSlot {
                runner: StackRunner::new(name, stack_bytes, home),
                taken: Arc::new(AtomicBool::new(false)),
            })
            .collect();
        Self { slots }
    }

    pub fn start<F>(&self, job: F) -> Result<(), RunError>
    where
        F: FnOnce() + Send + 'static,
    {
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.taken.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok())
            .ok_or(RunError::Busy)?;
        let release = Release(Arc::clone(&slot.taken));
        slot.runner.submit(
            Box::new(move || {
                let _release = release;
                job();
            }),
            Handover::OnlyIfRoom,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread::ThreadId;
    use std::time::{Duration, Instant};

    use super::{RunError, RunnerPool, StackRunner};
    use crate::esp_thread::StackHome;

    const TEST_STACK: usize = 64 * 1024;

    #[test]
    fn a_job_returns_its_result_to_the_caller() {
        static RUNNER: StackRunner = StackRunner::new(c"runner-result", TEST_STACK, StackHome::Internal);
        assert_eq!(RUNNER.run(|| 41 + 1).expect("the job ran"), 42);
    }

    #[test]
    fn every_job_runs_on_one_long_lived_thread_beside_the_caller() {
        static RUNNER: StackRunner = StackRunner::new(c"runner-reuse", TEST_STACK, StackHome::Internal);
        let first: ThreadId = RUNNER.run(|| std::thread::current().id()).expect("first job");
        let second: ThreadId = RUNNER.run(|| std::thread::current().id()).expect("second job");
        assert_eq!(first, second, "a second job spawned a thread of its own");
        assert_ne!(first, std::thread::current().id(), "the job ran on the caller's stack");
    }

    #[test]
    fn a_job_that_panics_fails_its_run_and_its_thread_runs_the_next_job() {
        static RUNNER: StackRunner = StackRunner::new(c"runner-panic", TEST_STACK, StackHome::Internal);
        let (seen, panicked_on) = mpsc::channel();
        let failed = RUNNER.run(move || -> u8 {
            let _ = seen.send(std::thread::current().id());
            panic!("the job failed")
        });
        assert!(matches!(failed, Err(RunError::Unanswered)));
        let next = RUNNER.run(|| std::thread::current().id()).expect("the runner took the next job");
        assert_eq!(panicked_on.recv().expect("the job ran"), next, "a panic cost the runner its thread");
    }

    #[test]
    fn callers_that_arrive_together_wait_their_turn() {
        static RUNNER: StackRunner = StackRunner::new(c"runner-turns", TEST_STACK, StackHome::Internal);
        const CALLERS: usize = 4;
        let (entered, first_inside) = mpsc::channel();
        let (release, held) = mpsc::channel::<()>();
        let blocker = std::thread::spawn(move || {
            RUNNER.run(move || {
                let _ = entered.send(());
                let _ = held.recv();
            })
        });
        first_inside.recv_timeout(JOB_DEADLINE).expect("the first job started");
        let (arriving, arrived) = mpsc::channel();
        let callers: Vec<_> = (0..CALLERS)
            .map(|n| {
                let arriving = arriving.clone();
                std::thread::spawn(move || {
                    let _ = arriving.send(());
                    RUNNER.run(move || n)
                })
            })
            .collect();
        for _ in 0..CALLERS {
            arrived.recv_timeout(JOB_DEADLINE).expect("a caller arrived");
        }
        drop(release);
        assert!(blocker.join().expect("first caller").is_ok());
        for (n, caller) in callers.into_iter().enumerate() {
            assert_eq!(caller.join().expect("caller").expect("a caller was turned away"), n);
        }
    }

    const POOL_RUNNERS: usize = 2;
    const JOBS_IN_TURN: usize = 3 * POOL_RUNNERS;
    const JOB_DEADLINE: Duration = Duration::from_secs(2);

    fn pool() -> RunnerPool {
        RunnerPool::new(c"pool-test", TEST_STACK, StackHome::Internal, POOL_RUNNERS)
    }

    fn try_start_recording(
        pool: &RunnerPool,
        threads: &Arc<Mutex<HashSet<ThreadId>>>,
    ) -> Result<mpsc::Receiver<()>, RunError> {
        let (done, finished) = mpsc::channel();
        let threads = Arc::clone(threads);
        pool.start(move || {
            threads.lock().unwrap().insert(std::thread::current().id());
            let _ = done.send(());
        })?;
        Ok(finished)
    }

    #[test]
    fn jobs_in_turn_reuse_the_pool_threads() {
        let pool = pool();
        let threads = Arc::new(Mutex::new(HashSet::new()));
        for _ in 0..JOBS_IN_TURN {
            start_recording_when_free(&pool, &threads);
        }
        let served = threads.lock().unwrap().len();
        assert!(served <= POOL_RUNNERS, "{served} threads served jobs in turn");
    }

    #[test]
    fn a_pool_with_every_runner_held_refuses_a_job_without_blocking() {
        let pool = pool();
        let (release, held) = mpsc::channel::<()>();
        let held = Arc::new(Mutex::new(held));
        let (started, running) = mpsc::channel();
        for _ in 0..POOL_RUNNERS {
            let held = Arc::clone(&held);
            let started = started.clone();
            pool.start(move || {
                let _ = started.send(());
                let _ = held.lock().unwrap().recv();
            })
            .expect("an idle runner took the long job");
            running.recv_timeout(JOB_DEADLINE).expect("the long job started");
        }
        assert!(matches!(pool.start(|| ()), Err(RunError::Busy)));
        drop(release);
        start_recording_when_free(&pool, &Arc::new(Mutex::new(HashSet::new())));
    }

    fn start_recording_when_free(pool: &RunnerPool, threads: &Arc<Mutex<HashSet<ThreadId>>>) {
        let deadline = Instant::now() + JOB_DEADLINE;
        loop {
            match try_start_recording(pool, threads) {
                Ok(finished) => {
                    finished.recv_timeout(JOB_DEADLINE).expect("the freed runner ran the job");
                    return;
                }
                Err(RunError::Busy) => assert!(Instant::now() < deadline, "no runner came free after its job ended"),
                Err(other) => panic!("the pool failed to start a job: {other}"),
            }
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_job_that_panics_frees_its_runner_and_keeps_its_thread() {
        let pool = RunnerPool::new(c"pool-panic", TEST_STACK, StackHome::Internal, 1);
        let threads = Arc::new(Mutex::new(HashSet::new()));
        let recorder = Arc::clone(&threads);
        pool.start(move || {
            recorder.lock().unwrap().insert(std::thread::current().id());
            panic!("the job failed")
        })
        .expect("the runner took the job");
        start_recording_when_free(&pool, &threads);
        assert_eq!(threads.lock().unwrap().len(), 1, "a panic cost the runner its thread");
    }
}
