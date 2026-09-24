use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dali2rust_platform::dali::{
    DaliTransport, ObservedFrameSender, ObservedRawFrame, ObservedRawFrameKind, TransferOutcome,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptedOutcome {
    NoBackward,
    Backward(u8),
    NoBackwardContended,
    BackwardContended(u8),
    SendError,
    ReceiveError,
    Collision,
    BusBusy,
    ForeignInWindow,
    CorruptedInWindow,
    NoAnswerContended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScriptedExchange {
    expected_frame: u16,
    outcome: ScriptedOutcome,
    foreign_before: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusFrameSource {
    Ours,
    Foreign,
}

#[derive(Debug)]
struct MockInner {
    sent_frames: Vec<u16>,
    sent_frames24: Vec<[u8; 3]>,
    frame24_answers: std::collections::HashMap<[u8; 3], u8>,
    frame24_outcomes: std::collections::VecDeque<TransferOutcome>,
    sent_frame_min_idle_us: Vec<u32>,
    sent_frame24_min_idle_us: Vec<u32>,
    sent_frame_times: Vec<Instant>,
    responses: VecDeque<u8>,
    persistent_response: Option<u8>,
    scripted_exchanges: VecDeque<ScriptedExchange>,
    scripted_pending_receive: Option<Result<Option<u8>, ()>>,
    script_error: Option<String>,
    pending_foreign_before: Option<u16>,
    foreign_frames_before: Vec<(usize, u16)>,
    block_send: bool,
    block_at_send: Option<usize>,
    blocked: Arc<AtomicBool>,
    block_timeout_ms: u64,
    foreign_activity: u32,
    observed_tx: Option<ObservedFrameSender>,
    dtr: [u8; 3],
    last_tx_settle_ticks: Option<u16>,
}

impl Default for MockInner {
    fn default() -> Self {
        Self {
            sent_frames: Vec::new(),
            sent_frames24: Vec::new(),
            frame24_answers: std::collections::HashMap::new(),
            frame24_outcomes: std::collections::VecDeque::new(),
            sent_frame_min_idle_us: Vec::new(),
            sent_frame24_min_idle_us: Vec::new(),
            sent_frame_times: Vec::new(),
            responses: VecDeque::new(),
            persistent_response: None,
            scripted_exchanges: VecDeque::new(),
            scripted_pending_receive: None,
            script_error: None,
            pending_foreign_before: None,
            foreign_frames_before: Vec::new(),
            block_send: false,
            block_at_send: None,
            blocked: Arc::new(AtomicBool::new(false)),
            block_timeout_ms: 60_000,
            foreign_activity: 0,
            observed_tx: None,
            dtr: [0; 3],
            last_tx_settle_ticks: None,
        }
    }
}

#[derive(Debug)]
pub struct MockDaliTransport {
    inner: Mutex<MockInner>,
    unblock_flag: Arc<AtomicBool>,
}

impl Default for MockDaliTransport {
    fn default() -> Self {
        let unblock_flag = Arc::new(AtomicBool::new(false));
        let mut inner = MockInner::default();
        inner.blocked = Arc::clone(&unblock_flag);
        Self {
            inner: Mutex::new(inner),
            unblock_flag,
        }
    }
}

impl MockDaliTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue_response(&self, value: u8) {
        self.inner.lock().unwrap().responses.push_back(value);
    }

    pub fn set_persistent_response(&self, value: u8) {
        self.inner.lock().unwrap().persistent_response = Some(value);
    }

    pub fn set_last_tx_settle_ticks(&self, ticks: Option<u16>) {
        self.inner.lock().unwrap().last_tx_settle_ticks = ticks;
    }

    pub fn expect_forward_frame(&self, frame: u16) {
        enqueue_scripted(&self.inner, frame, ScriptedOutcome::NoBackward);
    }

    pub fn expect_forward_frame_with_backward(&self, frame: u16, backward: Option<u8>) {
        let outcome = match backward {
            Some(value) => ScriptedOutcome::Backward(value),
            None => ScriptedOutcome::NoBackward,
        };
        enqueue_scripted(&self.inner, frame, outcome);
    }

    pub fn expect_forward_frame_with_backward_contended(&self, frame: u16, backward: Option<u8>) {
        let outcome = match backward {
            Some(value) => ScriptedOutcome::BackwardContended(value),
            None => ScriptedOutcome::NoBackwardContended,
        };
        enqueue_scripted(&self.inner, frame, outcome);
    }

    pub fn expect_forward_frame_send_error(&self, frame: u16) {
        enqueue_scripted(&self.inner, frame, ScriptedOutcome::SendError);
    }

    pub fn expect_forward_frame_receive_error(&self, frame: u16) {
        enqueue_scripted(&self.inner, frame, ScriptedOutcome::ReceiveError);
    }

    pub fn expect_forward_frame_collision(&self, frame: u16) {
        enqueue_scripted(&self.inner, frame, ScriptedOutcome::Collision);
    }

    pub fn expect_forward_frame_bus_busy(&self, frame: u16) {
        enqueue_scripted(&self.inner, frame, ScriptedOutcome::BusBusy);
    }

    pub fn expect_forward_frame_foreign_in_window(&self, frame: u16) {
        enqueue_scripted(&self.inner, frame, ScriptedOutcome::ForeignInWindow);
    }

    pub fn expect_forward_frame_corrupted_in_window(&self, frame: u16) {
        enqueue_scripted(&self.inner, frame, ScriptedOutcome::CorruptedInWindow);
    }

    pub fn expect_query_no_answer_contended(&self, frame: u16) {
        enqueue_scripted(&self.inner, frame, ScriptedOutcome::NoAnswerContended);
    }

    pub fn scripted_exchanges_remaining(&self) -> usize {
        self.inner.lock().unwrap().scripted_exchanges.len()
    }

    pub fn script_error(&self) -> Option<String> {
        self.inner.lock().unwrap().script_error.clone()
    }

    pub fn sent_frames(&self) -> Vec<u16> {
        self.inner.lock().unwrap().sent_frames.clone()
    }

    pub fn script_frame24_outcome(&self, outcome: TransferOutcome) {
        self.inner.lock().unwrap().frame24_outcomes.push_back(outcome);
    }

    pub fn script_frame24_answer(&self, frame: [u8; 3], answer: u8) {
        self.inner
            .lock()
            .unwrap()
            .frame24_answers
            .insert(frame, answer);
    }

    pub fn sent_frames24(&self) -> Vec<[u8; 3]> {
        self.inner.lock().unwrap().sent_frames24.clone()
    }

    pub fn clear_sent_frames24(&self) {
        let mut g = self.inner.lock().unwrap();
        g.sent_frames24.clear();
        g.sent_frame24_min_idle_us.clear();
    }

    pub fn clear_frame24_answers(&self) {
        self.inner.lock().unwrap().frame24_answers.clear();
    }

    pub fn sent_frame_settle_us(&self) -> Vec<u32> {
        self.inner.lock().unwrap().sent_frame_min_idle_us.clone()
    }

    pub fn sent_frame24_settle_us(&self) -> Vec<u32> {
        self.inner.lock().unwrap().sent_frame24_min_idle_us.clone()
    }

    pub fn forward_frame_gap_ms(&self, a: usize, b: usize) -> Option<u128> {
        let g = self.inner.lock().unwrap();
        let first = g.sent_frame_times.get(a)?;
        let second = g.sent_frame_times.get(b)?;
        Some(second.saturating_duration_since(*first).as_millis())
    }

    pub fn forward_frame_gaps_ms(&self) -> Vec<u128> {
        let g = self.inner.lock().unwrap();
        g.sent_frame_times
            .windows(2)
            .map(|pair| pair[1].saturating_duration_since(pair[0]).as_millis())
            .collect()
    }

    pub fn expect_foreign_frame_before_next(&self, foreign: u16) {
        self.inner.lock().unwrap().pending_foreign_before = Some(foreign);
    }

    pub fn foreign_frames_before(&self) -> Vec<(usize, u16)> {
        self.inner.lock().unwrap().foreign_frames_before.clone()
    }

    pub fn bus_trace(&self) -> Vec<(BusFrameSource, u16)> {
        let g = self.inner.lock().unwrap();
        let mut trace = Vec::with_capacity(g.sent_frames.len() + g.foreign_frames_before.len());
        for (index, frame) in g.sent_frames.iter().enumerate() {
            for (before, foreign) in &g.foreign_frames_before {
                if *before == index {
                    trace.push((BusFrameSource::Foreign, *foreign));
                }
            }
            trace.push((BusFrameSource::Ours, *frame));
        }
        trace
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock().unwrap();
        g.sent_frames.clear();
        g.sent_frames24.clear();
        g.sent_frame_min_idle_us.clear();
        g.sent_frame24_min_idle_us.clear();
        g.sent_frame_times.clear();
        g.responses.clear();
        g.persistent_response = None;
        g.scripted_exchanges.clear();
        g.scripted_pending_receive = None;
        g.script_error = None;
        g.pending_foreign_before = None;
        g.foreign_frames_before.clear();
        g.foreign_activity = 0;
    }

    pub fn set_foreign_activity(&self, value: u32) {
        self.inner.lock().unwrap().foreign_activity = value;
    }

    pub fn get_unblock_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.unblock_flag)
    }

    pub fn block_next_send(&self) {
        let mut g = self.inner.lock().unwrap();
        g.block_send = true;
        g.blocked.store(true, Ordering::Release);
    }

    pub fn block_next_send_for(&self, timeout_ms: u64) {
        let mut g = self.inner.lock().unwrap();
        g.block_send = true;
        g.block_timeout_ms = timeout_ms;
        g.blocked.store(true, Ordering::Release);
    }

    pub fn block_send_at(&self, nth: usize) {
        let mut g = self.inner.lock().unwrap();
        g.block_at_send = Some(nth);
        g.blocked.store(true, Ordering::Release);
    }

    pub fn unblock_send(&self) {
        let mut g = self.inner.lock().unwrap();
        g.block_send = false;
        self.unblock_flag.store(false, Ordering::Release);
        g.block_timeout_ms = 60_000;
    }
}

fn enqueue_scripted(inner: &Mutex<MockInner>, frame: u16, outcome: ScriptedOutcome) {
    let mut g = inner.lock().unwrap();
    let foreign_before = g.pending_foreign_before.take();
    g.scripted_exchanges.push_back(ScriptedExchange {
        expected_frame: frame,
        outcome,
        foreign_before,
    });
}

const DTR_WRITE_ADDRESS_BYTES: [u16; 3] = [0xA3, 0xC3, 0xC5];
const DTR_QUERY_OPCODES: [u16; 3] = [0x98, 0x9C, 0x9D];

fn record_forward_send(g: &mut MockInner, frame: u16, min_idle_us: u32) {
    g.sent_frames.push(frame);
    g.sent_frame_min_idle_us.push(min_idle_us);
    g.sent_frame_times.push(Instant::now());
    if let Some(index) = DTR_WRITE_ADDRESS_BYTES
        .iter()
        .position(|addr| *addr == frame >> 8)
    {
        g.dtr[index] = (frame & 0xFF) as u8;
    }
}

fn dtr_query_answer(g: &MockInner, frame: u16) -> Option<u8> {
    DTR_QUERY_OPCODES
        .iter()
        .position(|opcode| *opcode == frame & 0xFF)
        .map(|index| g.dtr[index])
}

fn check_scripted_frame(g: &mut MockInner, expected: u16, actual: u16) -> Result<(), ()> {
    if expected == actual {
        return Ok(());
    }
    g.script_error = Some(format!(
        "expected scripted frame 0x{expected:04X}, got 0x{actual:04X}",
    ));
    Err(())
}

fn fail_requires_exchange(g: &mut MockInner, outcome: ScriptedOutcome) -> Result<(), ()> {
    g.script_error = Some(format!(
        "scripted outcome {outcome:?} requires exchange_frame()"
    ));
    Err(())
}

fn note_foreign_activity(g: &mut MockInner) {
    g.foreign_activity = g.foreign_activity.saturating_add(1);
}

fn record_foreign_before(g: &mut MockInner, step: ScriptedExchange) {
    let Some(foreign) = step.foreign_before else {
        return;
    };
    let before_index = g.sent_frames.len();
    g.foreign_frames_before.push((before_index, foreign));
    note_foreign_activity(g);
}

fn backward_window_outcome(expects_backward: bool, in_window: TransferOutcome) -> TransferOutcome {
    if expects_backward {
        in_window
    } else {
        TransferOutcome::NoAnswer
    }
}

fn consume_scripted_step(
    g: &mut MockInner,
    frame: u16,
    min_idle_us: u32,
    step: ScriptedExchange,
) -> Result<(), ()> {
    check_scripted_frame(g, step.expected_frame, frame)?;
    record_foreign_before(g, step);
    match step.outcome {
        ScriptedOutcome::NoBackward => {
            record_forward_send(g, frame, min_idle_us);
            g.scripted_pending_receive = Some(Ok(None));
            Ok(())
        }
        ScriptedOutcome::Backward(value) => {
            record_forward_send(g, frame, min_idle_us);
            g.scripted_pending_receive = Some(Ok(Some(value)));
            Ok(())
        }
        ScriptedOutcome::SendError => {
            g.scripted_pending_receive = None;
            Err(())
        }
        ScriptedOutcome::ReceiveError => {
            record_forward_send(g, frame, min_idle_us);
            g.scripted_pending_receive = Some(Err(()));
            Ok(())
        }
        ScriptedOutcome::Collision
        | ScriptedOutcome::BusBusy
        | ScriptedOutcome::ForeignInWindow
        | ScriptedOutcome::CorruptedInWindow
        | ScriptedOutcome::NoBackwardContended
        | ScriptedOutcome::BackwardContended(_)
        | ScriptedOutcome::NoAnswerContended => fail_requires_exchange(g, step.outcome),
    }
}

fn scripted_exchange_outcome(
    g: &mut MockInner,
    frame: u16,
    expects_backward: bool,
    min_idle_us: u32,
    step: ScriptedExchange,
) -> Result<TransferOutcome, ()> {
    check_scripted_frame(g, step.expected_frame, frame)?;
    record_foreign_before(g, step);
    if step.outcome != ScriptedOutcome::BusBusy {
        record_forward_send(g, frame, min_idle_us);
    }
    scripted_transfer_outcome(g, expects_backward, step.outcome)
}

fn scripted_transfer_outcome(
    g: &mut MockInner,
    expects_backward: bool,
    outcome: ScriptedOutcome,
) -> Result<TransferOutcome, ()> {
    match outcome {
        ScriptedOutcome::NoBackward => Ok(TransferOutcome::NoAnswer),
        ScriptedOutcome::Backward(value) => Ok(TransferOutcome::Answer(value)),
        ScriptedOutcome::NoBackwardContended | ScriptedOutcome::NoAnswerContended => {
            note_foreign_activity(g);
            Ok(TransferOutcome::NoAnswer)
        }
        ScriptedOutcome::BackwardContended(value) => {
            note_foreign_activity(g);
            Ok(TransferOutcome::Answer(value))
        }
        ScriptedOutcome::SendError => Err(()),
        ScriptedOutcome::ReceiveError => {
            if expects_backward {
                Err(())
            } else {
                Ok(TransferOutcome::NoAnswer)
            }
        }
        ScriptedOutcome::Collision => Ok(TransferOutcome::Collision),
        ScriptedOutcome::BusBusy => Ok(TransferOutcome::BusBusy),
        ScriptedOutcome::ForeignInWindow => Ok(backward_window_outcome(
            expects_backward,
            TransferOutcome::ForeignInWindow,
        )),
        ScriptedOutcome::CorruptedInWindow => Ok(backward_window_outcome(
            expects_backward,
            TransferOutcome::CorruptedInWindow,
        )),
    }
}

impl MockDaliTransport {
    pub fn inject_observed_frame(&self, bytes: [u8; 3], kind: ObservedRawFrameKind) -> bool {
        let g = self.inner.lock().unwrap();
        let Some(tx) = g.observed_tx.as_ref() else {
            return false;
        };
        tx.try_send(ObservedRawFrame {
            bytes,
            kind,
            observed_at_ms: dali2rust_bsp::unix_clock::unix_wall_clock_millis(),
            observed_at_mono_ms: dali2rust_bsp::monotonic_clock::observation_stamp_ms(),
        })
        .is_ok()
    }
}

impl MockDaliTransport {
    fn wait_if_blocked<'a>(
        &'a self,
        g: std::sync::MutexGuard<'a, MockInner>,
    ) -> std::sync::MutexGuard<'a, MockInner> {
        if !(g.block_send || g.block_at_send.is_some_and(|n| g.sent_frames.len() == n)) {
            return g;
        }
        let blocked = Arc::clone(&g.blocked);
        let timeout_ms = g.block_timeout_ms;
        drop(g);
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while blocked.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                break;
            }
            // sleep-ok: mock blocked-send harness polls its own deadline
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut g = self.inner.lock().unwrap();
        g.block_send = false;
        g.block_at_send = None;
        g
    }
}

impl DaliTransport for MockDaliTransport {
    type Error = ();

    fn exchange_frame24(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
    ) -> Result<TransferOutcome, dali2rust_platform::dali::Frame24Error<Self::Error>> {
        self.exchange_frame24_with_settle(frame, expects_backward, 0)
    }

    fn exchange_frame24_with_settle(
        &mut self,
        frame: [u8; 3],
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, dali2rust_platform::dali::Frame24Error<Self::Error>> {
        let g = self.inner.lock().unwrap();
        let mut g = self.wait_if_blocked(g);
        g.sent_frames24.push(frame);
        g.sent_frame24_min_idle_us.push(min_idle_us);
        if let Some(outcome) = g.frame24_outcomes.pop_front() {
            return Ok(outcome);
        }
        if !expects_backward {
            return Ok(TransferOutcome::NoAnswer);
        }
        if let Some(answer) = g.frame24_answers.get(&frame) {
            return Ok(TransferOutcome::Answer(*answer));
        }
        let answer = g
            .responses
            .pop_front()
            .or(g.persistent_response)
            .map_or(TransferOutcome::NoAnswer, TransferOutcome::Answer);
        Ok(answer)
    }

    fn supports_frame24(&self) -> bool {
        true
    }

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Self::Error> {
        let g = self.inner.lock().unwrap();
        let mut g = self.wait_if_blocked(g);
        if let Some(step) = g.scripted_exchanges.pop_front() {
            consume_scripted_step(&mut g, frame, 0, step)?;
        } else {
            record_forward_send(&mut g, frame, 0);
        }
        Ok(())
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Self::Error> {
        let mut g = self.inner.lock().unwrap();
        if let Some(result) = g.scripted_pending_receive.take() {
            return result;
        }
        if let Some(v) = g.responses.pop_front() {
            return Ok(Some(v));
        }
        Ok(g.persistent_response)
    }

    fn is_bus_idle(&self) -> Result<bool, Self::Error> {
        Ok(true)
    }

    fn exchange_frame(
        &mut self,
        frame: u16,
        expects_backward: bool,
    ) -> Result<TransferOutcome, Self::Error> {
        self.exchange_frame_with_settle(frame, expects_backward, 0)
    }

    fn exchange_frame_with_settle(
        &mut self,
        frame: u16,
        expects_backward: bool,
        min_idle_us: u32,
    ) -> Result<TransferOutcome, Self::Error> {
        let g = self.inner.lock().unwrap();
        let mut g = self.wait_if_blocked(g);
        if let Some(step) = g.scripted_exchanges.pop_front() {
            return scripted_exchange_outcome(&mut g, frame, expects_backward, min_idle_us, step);
        }
        record_forward_send(&mut g, frame, min_idle_us);
        if !expects_backward {
            return Ok(TransferOutcome::NoAnswer);
        }
        if let Some(v) = g.responses.pop_front() {
            return Ok(TransferOutcome::Answer(v));
        }
        if g.persistent_response.is_some() {
            if let Some(v) = dtr_query_answer(&g, frame) {
                return Ok(TransferOutcome::Answer(v));
            }
        }
        Ok(match g.persistent_response {
            Some(v) => TransferOutcome::Answer(v),
            None => TransferOutcome::NoAnswer,
        })
    }

    fn last_tx_settle_ticks(&self) -> Option<u16> {
        self.inner.lock().unwrap().last_tx_settle_ticks
    }

    fn foreign_activity(&self) -> u32 {
        self.inner.lock().unwrap().foreign_activity
    }

    fn set_observed_frame_sender(&mut self, sender: ObservedFrameSender) {
        self.inner.lock().unwrap().observed_tx = Some(sender);
    }
}

#[cfg(test)]
#[allow(
    deprecated,
    reason = "backward compatibility with legacy BDD transport mock API"
)]
mod tests {
    use super::*;

    #[test]
    fn mock_transport_send_and_read() {
        let mut transport = MockDaliTransport::new();
        transport.enqueue_response(0x42);

        transport.send_forward_frame(0x02FE).unwrap();
        assert_eq!(transport.sent_frames(), vec![0x02FE]);

        let resp = transport.receive_backward_frame().unwrap();
        assert_eq!(resp, Some(0x42));
    }

    #[test]
    fn mock_transport_no_response() {
        let mut transport = MockDaliTransport::new();
        assert_eq!(transport.receive_backward_frame().unwrap(), None);
    }

    #[test]
    fn mock_transport_queue() {
        let mut transport = MockDaliTransport::new();
        transport.enqueue_response(1);
        transport.enqueue_response(2);
        transport.enqueue_response(3);

        assert_eq!(transport.receive_backward_frame().unwrap(), Some(1));
        assert_eq!(transport.receive_backward_frame().unwrap(), Some(2));
        assert_eq!(transport.receive_backward_frame().unwrap(), Some(3));
        assert_eq!(transport.receive_backward_frame().unwrap(), None);
    }

    #[test]
    fn mock_transport_clear() {
        let mut transport = MockDaliTransport::new();
        transport.send_forward_frame(0x1234).unwrap();
        transport.enqueue_response(0xFF);
        transport.clear();
        assert!(transport.sent_frames().is_empty());
        assert_eq!(transport.receive_backward_frame().unwrap(), None);
    }
}
