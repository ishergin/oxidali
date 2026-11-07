use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::frame::{
    header_tail_len, parse_header, server_header, unmask, FrameError, Opcode,
    MAX_SERVER_HEADER_BYTES, MAX_TAIL_BYTES, PRELUDE_BYTES,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    Closed,
    Timeout,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    Closed,
    Desync,
    Failed,
    Protocol(FrameError),
    Oversize,
    SecondReader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    Idle,
    Frame { opcode: Opcode, payload_len: usize },
}

pub trait WsWire: Send + Sync {
    fn read(&self, buf: &mut [u8]) -> Result<usize, WireError>;
    fn write_frame(&self, header: &[u8], payload: &[u8]) -> Result<(), WireError>;
    fn wake_reader(&self);
    fn finish(&self);
}

pub struct WsConnection<W: WsWire> {
    wire: W,
    writing: Mutex<()>,
    broken: AtomicBool,
    reading: AtomicBool,
}

impl<W: WsWire> WsConnection<W> {
    pub const fn new(wire: W) -> Self {
        Self {
            wire,
            writing: Mutex::new(()),
            broken: AtomicBool::new(false),
            reading: AtomicBool::new(false),
        }
    }

    pub fn send_frame(&self, opcode: Opcode, payload: &[u8]) -> Result<(), WireError> {
        if self.broken.load(Ordering::Acquire) {
            return Err(WireError::Closed);
        }
        let mut header = [0u8; MAX_SERVER_HEADER_BYTES];
        let header_len =
            server_header(opcode, payload.len(), &mut header).ok_or(WireError::Failed)?;
        let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        self.wire
            .write_frame(&header[..header_len], payload)
            .inspect_err(|_| {
                self.broken.store(true, Ordering::Release);
            })
    }

    pub fn is_broken(&self) -> bool {
        self.broken.load(Ordering::Acquire)
    }

    pub fn break_now(&self) {
        self.broken.store(true, Ordering::Release);
    }

    pub fn read_frame(&self, payload: &mut [u8]) -> Result<ReadOutcome, ReadError> {
        if self.reading.swap(true, Ordering::Acquire) {
            return Err(ReadError::SecondReader);
        }
        let outcome = self.read_one_frame(payload);
        self.reading.store(false, Ordering::Release);
        outcome
    }

    fn read_one_frame(&self, payload: &mut [u8]) -> Result<ReadOutcome, ReadError> {
        let mut prelude = [0u8; PRELUDE_BYTES];
        if !self.read_prelude(&mut prelude)? {
            return Ok(ReadOutcome::Idle);
        }
        let tail_len = header_tail_len(prelude).map_err(ReadError::Protocol)?;
        let mut tail = [0u8; MAX_TAIL_BYTES];
        self.read_exact(&mut tail[..tail_len])?;
        let header = parse_header(prelude, &tail[..tail_len]).map_err(ReadError::Protocol)?;
        if header.payload_len > payload.len() {
            return Err(ReadError::Oversize);
        }
        let body = &mut payload[..header.payload_len];
        self.read_exact(body)?;
        unmask(header.mask, body);
        Ok(ReadOutcome::Frame {
            opcode: header.opcode,
            payload_len: header.payload_len,
        })
    }

    pub fn wake_reader(&self) {
        self.wire.wake_reader();
    }

    fn read_prelude(&self, prelude: &mut [u8; PRELUDE_BYTES]) -> Result<bool, ReadError> {
        match self.wire.read(&mut prelude[..1]) {
            Ok(0) => return Err(ReadError::Closed),
            Ok(_) => {}
            Err(WireError::Timeout) => return Ok(false),
            Err(WireError::Closed) => return Err(ReadError::Closed),
            Err(WireError::Failed) => return Err(ReadError::Failed),
        }
        self.read_exact(&mut prelude[1..])?;
        Ok(true)
    }

    fn read_exact(&self, buf: &mut [u8]) -> Result<(), ReadError> {
        let mut filled = 0;
        while filled < buf.len() {
            filled += self.read_some(&mut buf[filled..])?;
        }
        Ok(())
    }

    fn read_some(&self, buf: &mut [u8]) -> Result<usize, ReadError> {
        match self.wire.read(buf) {
            Ok(0) => Err(ReadError::Closed),
            Ok(read) => Ok(read.min(buf.len())),
            Err(WireError::Timeout) => Err(ReadError::Desync),
            Err(WireError::Closed) => Err(ReadError::Closed),
            Err(WireError::Failed) => Err(ReadError::Failed),
        }
    }
}

impl<W: WsWire> Drop for WsConnection<W> {
    fn drop(&mut self) {
        self.wire.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::super::frame::MAX_CLIENT_FRAME_BYTES;
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    const BODY: usize = MAX_CLIENT_FRAME_BYTES;

    const KEY: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

    const INTRUSION_DEADLINE: Duration = Duration::from_millis(500);

    const ANNOUNCE_DEADLINE: Duration = Duration::from_secs(5);

    #[derive(Default)]
    struct Script {
        reads: Mutex<Vec<Result<Vec<u8>, WireError>>>,
        write_outcomes: Mutex<Vec<Result<(), WireError>>>,
        written: Mutex<Vec<Vec<u8>>>,
        finished: AtomicUsize,
        woken: AtomicUsize,
        inside: AtomicUsize,
        most_inside: AtomicUsize,
        frame_calls: AtomicUsize,
        hold_payloads: AtomicBool,
    }

    impl Script {
        fn reading(reads: Vec<Result<Vec<u8>, WireError>>) -> Arc<Self> {
            Arc::new(Self {
                reads: Mutex::new(reads),
                ..Self::default()
            })
        }

        fn writing(outcomes: Vec<Result<(), WireError>>) -> Arc<Self> {
            Arc::new(Self {
                write_outcomes: Mutex::new(outcomes),
                ..Self::default()
            })
        }

        fn silent() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn writes(&self) -> Vec<Vec<u8>> {
            self.written.lock().expect("the recorded writes").clone()
        }

        fn peak_inside(&self) -> usize {
            self.most_inside.load(Ordering::SeqCst)
        }

        fn frames_written(&self) -> usize {
            self.frame_calls.load(Ordering::SeqCst)
        }
    }

    struct Wire(Arc<Script>);

    impl Wire {
        fn hold_for_an_intruder(&self) {
            if !self.0.hold_payloads.load(Ordering::SeqCst) {
                return;
            }
            let until = Instant::now() + INTRUSION_DEADLINE;
            while Instant::now() < until && self.0.inside.load(Ordering::SeqCst) < 2 {
                std::thread::yield_now();
            }
        }
    }

    impl WsWire for Wire {
        fn read(&self, buf: &mut [u8]) -> Result<usize, WireError> {
            let mut reads = self.0.reads.lock().expect("the script");
            if reads.is_empty() {
                return Err(WireError::Closed);
            }
            let bytes = reads.remove(0)?;
            let take = bytes.len().min(buf.len());
            buf[..take].copy_from_slice(&bytes[..take]);
            if take < bytes.len() {
                reads.insert(0, Ok(bytes[take..].to_vec()));
            }
            Ok(take)
        }

        fn write_frame(&self, header: &[u8], payload: &[u8]) -> Result<(), WireError> {
            self.0.frame_calls.fetch_add(1, Ordering::SeqCst);
            let inside = self.0.inside.fetch_add(1, Ordering::SeqCst) + 1;
            self.0.most_inside.fetch_max(inside, Ordering::SeqCst);
            {
                let mut written = self.0.written.lock().expect("the writes");
                written.push(header.to_vec());
                if !payload.is_empty() {
                    written.push(payload.to_vec());
                }
            }
            self.hold_for_an_intruder();
            self.0.inside.fetch_sub(1, Ordering::SeqCst);
            let mut outcomes = self.0.write_outcomes.lock().expect("the outcomes");
            if outcomes.is_empty() {
                return Ok(());
            }
            outcomes.remove(0)
        }

        fn wake_reader(&self) {
            self.0.woken.fetch_add(1, Ordering::Relaxed);
        }

        fn finish(&self) {
            self.0.finished.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn conn(script: &Arc<Script>) -> WsConnection<Wire> {
        WsConnection::new(Wire(Arc::clone(script)))
    }

    fn masked(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0x80 | opcode, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&KEY);
        frame.extend(mask(payload));
        frame
    }

    fn masked_extended(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0x80 | opcode, 0x80 | 126];
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        frame.extend_from_slice(&KEY);
        frame.extend(mask(payload));
        frame
    }

    fn mask(payload: &[u8]) -> Vec<u8> {
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ KEY[index % 4])
            .collect()
    }

    #[test]
    fn a_masked_text_frame_arrives_unmasked_in_the_callers_buffer() {
        let script = Script::reading(vec![Ok(masked(0x1, b"hello"))]);
        let mut body = [0u8; BODY];
        let outcome = conn(&script).read_frame(&mut body).expect("a valid frame");
        assert_eq!(
            outcome,
            ReadOutcome::Frame {
                opcode: Opcode::Text,
                payload_len: 5
            }
        );
        assert_eq!(&body[..5], b"hello");
    }

    #[test]
    fn a_frame_with_an_extended_length_is_read_whole() {
        let payload = vec![b'x'; 300];
        let script = Script::reading(vec![Ok(masked_extended(0x1, &payload))]);
        let mut body = [0u8; BODY];
        let outcome = conn(&script)
            .read_frame(&mut body)
            .expect("a 300-byte frame is under the ceiling");
        assert_eq!(
            outcome,
            ReadOutcome::Frame {
                opcode: Opcode::Text,
                payload_len: 300
            },
            "a 126 marker needs two more length bytes before the key"
        );
        assert_eq!(&body[..300], &payload[..]);
    }

    #[test]
    fn a_payload_exactly_filling_the_buffer_is_read() {
        let payload = vec![b'y'; BODY];
        let script = Script::reading(vec![Ok(masked_extended(0x1, &payload))]);
        let mut body = [0u8; BODY];
        let outcome = conn(&script).read_frame(&mut body).expect("the ceiling fits");
        assert_eq!(
            outcome,
            ReadOutcome::Frame {
                opcode: Opcode::Text,
                payload_len: BODY
            }
        );
    }

    #[test]
    fn a_frame_split_across_reads_is_reassembled() {
        let whole = masked(0x1, b"subscribe");
        let script = Script::reading(whole.iter().map(|b| Ok(vec![*b])).collect());
        let mut body = [0u8; BODY];
        let outcome = conn(&script).read_frame(&mut body).expect("a split frame");
        assert_eq!(
            outcome,
            ReadOutcome::Frame {
                opcode: Opcode::Text,
                payload_len: 9
            }
        );
        assert_eq!(&body[..9], b"subscribe");
    }

    #[test]
    fn a_timeout_before_the_first_byte_is_idle_not_an_error() {
        let script = Script::reading(vec![Err(WireError::Timeout)]);
        let mut body = [0u8; BODY];
        assert_eq!(
            conn(&script).read_frame(&mut body).expect("an idle turn"),
            ReadOutcome::Idle
        );
    }

    #[test]
    fn a_timeout_after_the_first_byte_desyncs_rather_than_restarting() {
        for cut in 1..=2 {
            let whole = masked(0x1, b"hi");
            let mut reads: Vec<Result<Vec<u8>, WireError>> =
                whole[..cut].iter().map(|b| Ok(vec![*b])).collect();
            reads.push(Err(WireError::Timeout));
            let script = Script::reading(reads);
            let mut body = [0u8; BODY];
            assert_eq!(
                conn(&script).read_frame(&mut body),
                Err(ReadError::Desync),
                "a timeout {cut} byte(s) in must not look like a fresh header"
            );
        }
    }

    #[test]
    fn a_wire_that_reports_closed_is_never_read_as_idle() {
        for at in 0..3 {
            let whole = masked(0x1, b"hi");
            let mut reads: Vec<Result<Vec<u8>, WireError>> =
                whole[..at].iter().map(|b| Ok(vec![*b])).collect();
            reads.push(Err(WireError::Closed));
            let script = Script::reading(reads);
            let mut body = [0u8; BODY];
            assert_eq!(
                conn(&script).read_frame(&mut body),
                Err(ReadError::Closed),
                "a closed wire {at} byte(s) in was reported as something else"
            );
        }
    }

    #[test]
    fn a_wire_that_reports_a_failure_is_told_from_a_close() {
        for at in 0..3 {
            let whole = masked(0x1, b"hi");
            let mut reads: Vec<Result<Vec<u8>, WireError>> =
                whole[..at].iter().map(|b| Ok(vec![*b])).collect();
            reads.push(Err(WireError::Failed));
            let script = Script::reading(reads);
            let mut body = [0u8; BODY];
            assert_eq!(
                conn(&script).read_frame(&mut body),
                Err(ReadError::Failed),
                "a failing wire {at} byte(s) in was reported as something else"
            );
        }
    }

    #[test]
    fn an_end_of_stream_is_a_close() {
        let script = Script::reading(vec![Ok(Vec::new())]);
        let mut body = [0u8; BODY];
        assert_eq!(conn(&script).read_frame(&mut body), Err(ReadError::Closed));
    }

    #[test]
    fn a_protocol_violation_is_reported_before_any_payload_is_read() {
        let script = Script::reading(vec![Ok(vec![0x81, 0x05, 1, 2, 3, 4, 5])]);
        let mut body = [0u8; BODY];
        assert_eq!(
            conn(&script).read_frame(&mut body),
            Err(ReadError::Protocol(FrameError::Unmasked))
        );
    }

    #[test]
    fn a_payload_larger_than_the_buffer_is_refused_rather_than_overrunning_it() {
        let script = Script::reading(vec![Ok(masked(0x1, b"0123456789"))]);
        let mut body = [0u8; 4];
        assert_eq!(
            conn(&script).read_frame(&mut body),
            Err(ReadError::Oversize)
        );
    }

    #[test]
    fn an_over_reporting_wire_cannot_make_the_reader_see_bytes_it_never_got() {
        struct Liar {
            frame: Mutex<Vec<u8>>,
        }
        impl WsWire for Liar {
            fn read(&self, buf: &mut [u8]) -> Result<usize, WireError> {
                let mut frame = self.frame.lock().expect("the frame");
                if frame.is_empty() {
                    return Err(WireError::Closed);
                }
                let take = frame.len().min(buf.len());
                buf[..take].copy_from_slice(&frame[..take]);
                frame.drain(..take);
                Ok(usize::MAX)
            }
            fn write_frame(&self, _header: &[u8], _payload: &[u8]) -> Result<(), WireError> {
                Ok(())
            }
            fn wake_reader(&self) {}
            fn finish(&self) {}
        }

        let payload = vec![b'z'; 300];
        let liar = || Liar {
            frame: Mutex::new(masked_extended(0x1, &payload)),
        };

        let mut body = [0u8; BODY];
        assert_eq!(
            WsConnection::new(liar()).read_frame(&mut body),
            Ok(ReadOutcome::Frame {
                opcode: Opcode::Text,
                payload_len: 300
            }),
            "a wire claiming more than it gave must not stretch the frame"
        );
        assert_eq!(&body[..300], &payload[..]);

        let mut small = [0u8; 4];
        assert_eq!(
            WsConnection::new(liar()).read_frame(&mut small),
            Err(ReadError::Oversize),
            "the buffer still bounds the frame, whatever the wire claims"
        );
    }

    #[test]
    fn a_second_reader_is_refused_rather_than_splitting_a_frame() {
        struct Parked {
            entered: std::sync::mpsc::Sender<()>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl WsWire for Parked {
            fn read(&self, _buf: &mut [u8]) -> Result<usize, WireError> {
                let Ok(release) = self.release.try_lock() else {
                    return Err(WireError::Failed);
                };
                let _ = self.entered.send(());
                let _ = release.recv();
                Err(WireError::Closed)
            }
            fn write_frame(&self, _header: &[u8], _payload: &[u8]) -> Result<(), WireError> {
                Ok(())
            }
            fn wake_reader(&self) {}
            fn finish(&self) {}
        }

        let (entered, inside) = std::sync::mpsc::channel();
        let (release, held) = std::sync::mpsc::channel();
        let shared = Arc::new(WsConnection::new(Parked {
            entered,
            release: Mutex::new(held),
        }));
        let first = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let mut body = [0u8; BODY];
                shared.read_frame(&mut body)
            })
        };
        inside
            .recv_timeout(ANNOUNCE_DEADLINE)
            .expect("the first reader is inside read_frame");

        let mut body = [0u8; BODY];
        let second = shared.read_frame(&mut body);
        let _ = release.send(());
        let _ = first.join();
        assert_eq!(
            second,
            Err(ReadError::SecondReader),
            "two readers on one socket must not both be served"
        );
    }

    #[test]
    fn a_sent_frame_is_its_header_then_its_payload() {
        let script = Script::silent();
        conn(&script)
            .send_frame(Opcode::Text, b"{}")
            .expect("the frame went out");
        assert_eq!(script.writes(), vec![vec![0x81, 0x02], b"{}".to_vec()]);
    }

    #[test]
    fn an_empty_frame_writes_its_header_alone() {
        let script = Script::silent();
        conn(&script)
            .send_frame(Opcode::Close, &[])
            .expect("the close went out");
        assert_eq!(script.writes(), vec![vec![0x88, 0x00]]);
    }

    #[test]
    fn a_payload_needing_a_64_bit_length_fails_without_writing_a_byte() {
        let script = Script::silent();
        let huge = vec![0u8; usize::from(u16::MAX) + 1];
        assert_eq!(
            conn(&script).send_frame(Opcode::Text, &huge),
            Err(WireError::Failed)
        );
        assert!(
            script.writes().is_empty(),
            "a refused frame must not leave a header on the wire"
        );
        assert!(
            !conn(&script).is_broken(),
            "a frame refused before any byte went out is not a broken stream"
        );
    }

    #[test]
    fn a_failed_payload_write_breaks_the_connection_for_good() {
        let script = Script::writing(vec![Err(WireError::Timeout)]);
        let connection = conn(&script);
        assert_eq!(
            connection.send_frame(Opcode::Text, b"abc"),
            Err(WireError::Timeout)
        );
        assert!(
            connection.is_broken(),
            "a frame abandoned half-written leaves the stream desynced"
        );
        let before = script.writes().len();
        assert_eq!(
            connection.send_frame(Opcode::Close, &[]),
            Err(WireError::Closed),
            "a courtesy close must not be written into an unfinished frame"
        );
        assert_eq!(
            script.writes().len(),
            before,
            "nothing more may reach a broken wire"
        );
    }

    #[test]
    fn a_frame_reaches_the_wire_as_one_call_so_nothing_can_land_inside_it() {
        let script = Script::silent();
        conn(&script)
            .send_frame(Opcode::Text, b"abc")
            .expect("the frame went out");
        assert_eq!(
            script.frames_written(),
            1,
            "a frame split into two wire calls can have another frame written between them"
        );
    }

    #[test]
    fn a_second_sender_cannot_enter_while_a_frame_is_half_written() {
        let script = Script::silent();
        script.hold_payloads.store(true, Ordering::SeqCst);
        let shared = Arc::new(conn(&script));

        let intruder = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                shared.send_frame(Opcode::Text, b"bbb").expect("the frame");
            })
        };
        shared.send_frame(Opcode::Text, b"aaa").expect("the frame");
        intruder.join().expect("the second sender");

        assert_eq!(
            script.peak_inside(),
            1,
            "a second sender wrote while the first frame was still unfinished"
        );
        let writes = script.writes();
        assert_eq!(writes.len(), 4);
        for pair in writes.chunks(2) {
            assert_eq!(
                pair[0],
                vec![0x81, 0x03],
                "a payload landed where a header belongs"
            );
            assert!(pair[1].iter().all(|byte| *byte == pair[1][0]));
        }
    }

    #[test]
    fn the_wire_is_finished_once_when_its_last_owner_goes() {
        let script = Script::silent();
        let shared = Arc::new(conn(&script));
        let second = Arc::clone(&shared);
        drop(shared);
        assert_eq!(
            script.finished.load(Ordering::Relaxed),
            0,
            "the wire was finished while an owner was still holding it"
        );
        drop(second);
        assert_eq!(script.finished.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn waking_the_reader_reaches_the_wire() {
        let script = Script::silent();
        conn(&script).wake_reader();
        assert_eq!(script.woken.load(Ordering::Relaxed), 1);
    }
}
