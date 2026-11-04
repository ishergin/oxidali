use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffOutcome {
    Done,
    Failed,
    Refused,
    Lost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Queued,
    Running,
    Done(bool),
    Lost,
}

const fn waited_out(phase: Phase) -> bool {
    matches!(phase, Phase::Queued)
}

struct Pending<P> {
    phase: Phase,
    work: Option<P>,
}

pub struct SendSlot<P> {
    pending: Mutex<Pending<P>>,
    settled: Condvar,
    held: AtomicBool,
    deadline: Duration,
}

impl<P> SendSlot<P> {
    fn new(deadline: Duration) -> Self {
        Self {
            pending: Mutex::new(Pending { phase: Phase::Idle, work: None }),
            settled: Condvar::new(),
            held: AtomicBool::new(false),
            deadline,
        }
    }

    pub fn hand_over(&self, work: P, submit: impl FnOnce() -> bool) -> HandoffOutcome {
        if !self.arm(work) {
            return HandoffOutcome::Refused;
        }
        if !submit() {
            return self.disarm();
        }
        let Ok(pending) = self.pending.lock() else {
            return HandoffOutcome::Failed;
        };
        self.wait_for_outcome(pending)
    }

    fn arm(&self, work: P) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        if pending.phase != Phase::Idle {
            return false;
        }
        pending.work = Some(work);
        pending.phase = Phase::Queued;
        true
    }

    fn disarm(&self) -> HandoffOutcome {
        if let Ok(mut pending) = self.pending.lock() {
            pending.phase = Phase::Idle;
            pending.work = None;
        }
        HandoffOutcome::Refused
    }

    fn wait_for_outcome(&self, pending: MutexGuard<'_, Pending<P>>) -> HandoffOutcome {
        let waiting = |p: &mut Pending<P>| waited_out(p.phase);
        let Ok((mut pending, timed_out)) =
            self.settled
                .wait_timeout_while(pending, self.deadline, waiting)
        else {
            return HandoffOutcome::Failed;
        };
        if timed_out.timed_out() {
            pending.phase = Phase::Lost;
            pending.work = None;
            return HandoffOutcome::Lost;
        }
        self.wait_for_the_executor(pending)
    }

    fn wait_for_the_executor(&self, pending: MutexGuard<'_, Pending<P>>) -> HandoffOutcome {
        let running = |p: &mut Pending<P>| p.phase == Phase::Running;
        let Ok(mut pending) = self.settled.wait_while(pending, running) else {
            return HandoffOutcome::Failed;
        };
        let Phase::Done(sent) = pending.phase else {
            return HandoffOutcome::Failed;
        };
        pending.phase = Phase::Idle;
        if sent {
            HandoffOutcome::Done
        } else {
            HandoffOutcome::Failed
        }
    }

    pub fn execute(&self, run: impl FnOnce(P) -> bool) -> bool {
        let Some(work) = self.take_work() else {
            self.finish(false);
            return false;
        };
        let sent = run(work);
        self.finish(sent);
        true
    }

    fn take_work(&self) -> Option<P> {
        let mut pending = self.pending.lock().ok()?;
        if pending.phase != Phase::Queued {
            return None;
        }
        pending.phase = Phase::Running;
        pending.work.take()
    }

    fn finish(&self, sent: bool) {
        let Ok(mut pending) = self.pending.lock() else {
            return;
        };
        if pending.phase == Phase::Running {
            pending.phase = Phase::Done(sent);
        } else {
            pending.phase = Phase::Idle;
            pending.work = None;
        }
        self.settled.notify_one();
    }

    pub fn release(&self) {
        self.held.store(false, Ordering::Release);
    }

    fn take(&self) -> bool {
        if !self.pending.lock().is_ok_and(|p| p.phase == Phase::Idle) {
            return false;
        }
        self.held
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn is_lost(&self) -> bool {
        self.pending.lock().is_ok_and(|p| p.phase == Phase::Lost)
    }
}

pub struct SlotPool<P> {
    slots: Vec<SendSlot<P>>,
}

impl<P> SlotPool<P> {
    pub fn new(slots: usize, deadline: Duration) -> Self {
        Self {
            slots: (0..slots).map(|_| SendSlot::new(deadline)).collect(),
        }
    }

    pub fn claim(&self) -> Option<&SendSlot<P>> {
        self.slots.iter().find(|slot| slot.take())
    }

    pub fn lost(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_lost()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    const DEADLINE: Duration = Duration::from_millis(200);

    const WORK: u8 = 7;

    fn pool(slots: usize) -> &'static SlotPool<u8> {
        // SAFETY: test-only leak; a slot handed to a foreign executor must outlive it.
        Box::leak(Box::new(SlotPool::new(slots, DEADLINE)))
    }

    fn executor(rx: mpsc::Receiver<&'static SendSlot<u8>>) -> thread::JoinHandle<bool> {
        thread::spawn(move || {
            let slot = rx.recv().expect("the submitted slot");
            slot.execute(|work| work == WORK)
        })
    }

    #[test]
    fn work_the_executor_runs_reports_its_outcome() {
        let pool = pool(1);
        let slot = pool.claim().expect("a free slot");
        let (tx, rx) = mpsc::channel();
        let executor = executor(rx);
        assert_eq!(
            slot.hand_over(WORK, || tx.send(slot).is_ok()),
            HandoffOutcome::Done
        );
        assert!(executor.join().expect("the executor"), "the work never ran");
        assert!(pool.claim().is_none(), "the slot is still held by its sink");
        slot.release();
        assert!(pool.claim().is_some(), "a settled slot returns to the pool");
    }

    #[test]
    fn work_the_executor_never_runs_stops_waiting_at_the_deadline() {
        let pool = pool(1);
        let slot = pool.claim().expect("a free slot");
        assert_eq!(slot.hand_over(WORK, || true), HandoffOutcome::Lost);
    }

    #[test]
    fn only_work_still_waiting_to_be_taken_is_ever_waited_out() {
        assert!(waited_out(Phase::Queued), "a queued item may never have landed");
        assert!(
            !waited_out(Phase::Running),
            "an item the executor took is being served, however slowly; the deadline \
             is there for the one lost on the way"
        );
        for settled in [Phase::Idle, Phase::Done(true), Phase::Done(false), Phase::Lost] {
            assert!(!waited_out(settled), "{settled:?} is not a wait");
        }
    }

    #[test]
    fn work_refused_outright_leaves_the_slot_usable() {
        let pool = pool(1);
        let slot = pool.claim().expect("a free slot");
        assert_eq!(slot.hand_over(WORK, || false), HandoffOutcome::Refused);
        let (tx, rx) = mpsc::channel();
        let executor = executor(rx);
        assert_eq!(
            slot.hand_over(WORK, || tx.send(slot).is_ok()),
            HandoffOutcome::Done
        );
        assert!(executor.join().expect("the executor"));
    }

    #[test]
    fn a_lost_slot_is_freed_by_the_work_that_lands_late_and_sends_nothing() {
        let pool = pool(1);
        let slot = pool.claim().expect("a free slot");
        assert_eq!(slot.hand_over(WORK, || true), HandoffOutcome::Lost);
        slot.release();
        assert_eq!(pool.lost(), 1);
        assert!(
            pool.claim().is_none(),
            "a lost slot must not be handed to the next client"
        );
        assert!(
            !slot.execute(|_| true),
            "work that lands after the deadline must not be sent"
        );
        assert_eq!(pool.lost(), 0);
        assert!(pool.claim().is_some(), "the late work frees its slot");
    }

    #[test]
    fn a_sink_whose_work_was_lost_is_refused_rather_than_wedged() {
        let pool = pool(1);
        let slot = pool.claim().expect("a free slot");
        assert_eq!(slot.hand_over(WORK, || true), HandoffOutcome::Lost);
        assert_eq!(slot.hand_over(WORK, || true), HandoffOutcome::Refused);
    }

    #[test]
    fn each_sink_claims_a_slot_of_its_own_until_the_pool_runs_out() {
        let pool = pool(2);
        let first = pool.claim().expect("a free slot");
        let second = pool.claim().expect("a second free slot");
        assert!(!std::ptr::eq(first, second), "two sinks share one slot");
        assert!(pool.claim().is_none(), "the pool handed out a third slot");
        first.release();
        assert!(pool.claim().is_some(), "a released slot comes back");
    }
}
