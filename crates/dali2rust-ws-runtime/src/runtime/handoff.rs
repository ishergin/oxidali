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
        let Ok(mut pending) = self.pending.lock() else {
            return HandoffOutcome::Failed;
        };
        if pending.phase != Phase::Idle {
            return HandoffOutcome::Refused;
        }
        pending.work = Some(work);
        pending.phase = Phase::Queued;
        if !submit() {
            pending.phase = Phase::Idle;
            pending.work = None;
            return HandoffOutcome::Refused;
        }
        self.wait_for_outcome(pending)
    }

    fn wait_for_outcome(&self, pending: MutexGuard<'_, Pending<P>>) -> HandoffOutcome {
        let waiting = |p: &mut Pending<P>| matches!(p.phase, Phase::Queued | Phase::Running);
        let Ok((mut pending, _)) =
            self.settled
                .wait_timeout_while(pending, self.deadline, waiting)
        else {
            return HandoffOutcome::Failed;
        };
        if let Phase::Done(sent) = pending.phase {
            pending.phase = Phase::Idle;
            return if sent { HandoffOutcome::Done } else { HandoffOutcome::Failed };
        }
        pending.phase = Phase::Lost;
        pending.work = None;
        HandoffOutcome::Lost
    }

    pub fn take_work(&self) -> Option<P> {
        let mut pending = self.pending.lock().ok()?;
        if pending.phase != Phase::Queued {
            return None;
        }
        pending.phase = Phase::Running;
        pending.work.take()
    }

    pub fn finish(&self, sent: bool) {
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
        self.slots
            .iter()
            .filter(|slot| slot.pending.lock().is_ok_and(|p| p.phase == Phase::Lost))
            .count()
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

    #[test]
    fn work_the_executor_runs_reports_its_outcome() {
        let pool = pool(1);
        let slot = pool.claim().expect("a free slot");
        let (tx, rx) = mpsc::channel();
        let executor = thread::spawn(move || {
            let slot: &'static SendSlot<u8> = rx.recv().expect("the submitted slot");
            let sent = slot.take_work() == Some(WORK);
            slot.finish(sent);
        });
        assert_eq!(
            slot.hand_over(WORK, || tx.send(slot).is_ok()),
            HandoffOutcome::Done
        );
        executor.join().expect("the executor");
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
    fn work_refused_outright_leaves_the_slot_usable() {
        let pool = pool(1);
        let slot = pool.claim().expect("a free slot");
        assert_eq!(slot.hand_over(WORK, || false), HandoffOutcome::Refused);
        let (tx, rx) = mpsc::channel();
        let executor = thread::spawn(move || {
            let slot: &'static SendSlot<u8> = rx.recv().expect("the submitted slot");
            let sent = slot.take_work().is_some();
            slot.finish(sent);
        });
        assert_eq!(
            slot.hand_over(WORK, || tx.send(slot).is_ok()),
            HandoffOutcome::Done
        );
        executor.join().expect("the executor");
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
            slot.take_work().is_none(),
            "work that lands after the deadline must not be sent"
        );
        slot.finish(false);
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
