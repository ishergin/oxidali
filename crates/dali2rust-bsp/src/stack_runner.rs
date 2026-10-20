use core::ffi::CStr;
use std::sync::mpsc::{self, Receiver, SendError, SyncSender};
use std::sync::{Mutex, PoisonError};

use crate::esp_thread::{try_spawn_named_stack_in, StackHome};

type Job = Box<dyn FnOnce() + Send + 'static>;

const QUEUED_JOBS: usize = 1;
const ANSWER_SLOTS: usize = 1;

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

    pub fn run<R, F>(&self, job: F) -> std::io::Result<R>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        let (answer, answered) = mpsc::sync_channel(ANSWER_SLOTS);
        self.submit(Box::new(move || {
            let _ = answer.send(job());
        }))?;
        answered.recv().map_err(|_| {
            self.retire();
            std::io::Error::other("the job ended without an answer")
        })
    }

    fn retire(&self) {
        *self.jobs.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn submit(&self, job: Job) -> std::io::Result<()> {
        let mut jobs = self.jobs.lock().unwrap_or_else(PoisonError::into_inner);
        let job = match jobs.as_ref() {
            Some(queue) => match queue.send(job) {
                Ok(()) => return Ok(()),
                Err(SendError(job)) => job,
            },
            None => job,
        };
        let queue = self.spawn()?;
        queue
            .send(job)
            .map_err(|_| std::io::Error::other("the runner thread ended before its first job"))?;
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
        job();
    }
}

#[cfg(test)]
mod tests {
    use std::thread::ThreadId;

    use super::StackRunner;
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
    fn a_job_that_panics_fails_its_run_and_the_next_job_still_runs() {
        static RUNNER: StackRunner = StackRunner::new(c"runner-panic", TEST_STACK, StackHome::Internal);
        assert!(RUNNER.run(|| -> u8 { panic!("the job failed") }).is_err());
        assert_eq!(RUNNER.run(|| 7u8).expect("a fresh runner thread took the job"), 7);
    }
}
