use std::sync::Mutex;

use super::frame::{
    header_tail_len, parse_header, server_header, unmask, FrameError, Opcode, MAX_SERVER_HEADER_BYTES,
    MAX_TAIL_BYTES, PRELUDE_BYTES,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOutcome {
    Idle,
    Frame { opcode: Opcode, payload_len: usize },
}

pub trait WsWire: Send + Sync {
    fn read(&self, buf: &mut [u8]) -> Result<usize, WireError>;
    fn write_all(&self, buf: &[u8]) -> Result<(), WireError>;
    fn wake_reader(&self);
    fn finish(&self);
}

pub struct WsConnection<W: WsWire> {
    wire: W,
    writing: Mutex<()>,
}

impl<W: WsWire> WsConnection<W> {
    pub const fn new(wire: W) -> Self {
        Self {
            wire,
            writing: Mutex::new(()),
        }
    }

    pub fn send_frame(&self, opcode: Opcode, payload: &[u8]) -> Result<(), WireError> {
        let mut header = [0u8; MAX_SERVER_HEADER_BYTES];
        let header_len = server_header(opcode, payload.len(), &mut header)
            .ok_or(WireError::Failed)?;
        let _writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        self.wire.write_all(&header[..header_len])?;
        if payload.is_empty() {
            return Ok(());
        }
        self.wire.write_all(payload)
    }

    pub fn read_frame(&self, payload: &mut [u8]) -> Result<ReadOutcome, ReadError> {
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
            Ok(read) => Ok(read),
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
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const BODY: usize = 512;

    const ANNOUNCE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

    const INTRUSION_WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

    #[derive(Default)]
    struct Script {
        reads: Mutex<Vec<Result<Vec<u8>, WireError>>>,
        written: Mutex<Vec<Vec<u8>>>,
        finished: AtomicUsize,
        woken: AtomicUsize,
        inside: AtomicUsize,
        most_inside: AtomicUsize,
        hold_first_write: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    }

    impl Script {
        fn with(reads: Vec<Result<Vec<u8>, WireError>>) -> Arc<Self> {
            Arc::new(Self {
                reads: Mutex::new(reads),
                ..Self::default()
            })
        }

        fn writes(&self) -> Vec<Vec<u8>> {
            self.written.lock().expect("the recorded writes").clone()
        }
    }

    struct Wire(Arc<Script>);

    impl Wire {
        fn hold_if_asked(&self) {
            let held = self.0.hold_first_write.lock().expect("the hold").take();
            if let Some(announced) = held {
                let _ = announced.recv_timeout(ANNOUNCE_DEADLINE);
                std::thread::sleep(INTRUSION_WINDOW);
            }
        }
    }

    impl WsWire for Wire {
        fn read(&self, buf: &mut [u8]) -> Result<usize, WireError> {
            let mut reads = self.0.reads.lock().expect("the script");
            if reads.is_empty() {
                return Err(WireError::Closed);
            }
            match reads.remove(0) {
                Err(err) => Err(err),
                Ok(bytes) => {
                    let take = bytes.len().min(buf.len());
                    buf[..take].copy_from_slice(&bytes[..take]);
                    if take < bytes.len() {
                        reads.insert(0, Ok(bytes[take..].to_vec()));
                    }
                    Ok(take)
                }
            }
        }

        fn write_all(&self, buf: &[u8]) -> Result<(), WireError> {
            let inside = self.0.inside.fetch_add(1, Ordering::SeqCst) + 1;
            self.0.most_inside.fetch_max(inside, Ordering::SeqCst);
            self.0.written.lock().expect("the writes").push(buf.to_vec());
            self.hold_if_asked();
            self.0.inside.fetch_sub(1, Ordering::SeqCst);
            Ok(())
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

    fn masked_text(payload: &[u8], key: [u8; 4]) -> Vec<u8> {
        let mut frame = vec![0x81, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&key);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ key[index % 4]),
        );
        frame
    }

    #[test]
    fn a_masked_text_frame_arrives_unmasked_in_the_callers_buffer() {
        let script = Script::with(vec![Ok(masked_text(b"hello", [1, 2, 3, 4]))]);
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
    fn a_frame_split_across_reads_is_reassembled() {
        let whole = masked_text(b"subscribe", [9, 8, 7, 6]);
        let script = Script::with(whole.iter().map(|b| Ok(vec![*b])).collect());
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
        let script = Script::with(vec![Err(WireError::Timeout)]);
        let mut body = [0u8; BODY];
        assert_eq!(
            conn(&script).read_frame(&mut body).expect("an idle turn"),
            ReadOutcome::Idle
        );
    }

    #[test]
    fn a_timeout_after_the_first_byte_desyncs_rather_than_restarting() {
        for cut in 1..=2 {
            let whole = masked_text(b"hi", [1, 1, 1, 1]);
            let mut reads: Vec<Result<Vec<u8>, WireError>> =
                whole[..cut].iter().map(|b| Ok(vec![*b])).collect();
            reads.push(Err(WireError::Timeout));
            let script = Script::with(reads);
            let mut body = [0u8; BODY];
            assert_eq!(
                conn(&script).read_frame(&mut body),
                Err(ReadError::Desync),
                "a timeout {cut} byte(s) in must not look like a fresh header"
            );
        }
    }

    #[test]
    fn a_closed_socket_is_told_from_a_timeout() {
        let script = Script::with(vec![Ok(Vec::new())]);
        let mut body = [0u8; BODY];
        assert_eq!(conn(&script).read_frame(&mut body), Err(ReadError::Closed));
    }

    #[test]
    fn a_protocol_violation_is_reported_before_any_payload_is_read() {
        let script = Script::with(vec![Ok(vec![0x81, 0x05, 1, 2, 3, 4, 5])]);
        let mut body = [0u8; BODY];
        assert_eq!(
            conn(&script).read_frame(&mut body),
            Err(ReadError::Protocol(FrameError::Unmasked))
        );
    }

    #[test]
    fn a_payload_larger_than_the_buffer_is_refused_rather_than_overrunning_it() {
        let script = Script::with(vec![Ok(masked_text(b"0123456789", [1, 2, 3, 4]))]);
        let mut body = [0u8; 4];
        assert_eq!(
            conn(&script).read_frame(&mut body),
            Err(ReadError::Oversize)
        );
    }

    #[test]
    fn a_sent_frame_is_its_header_then_its_payload() {
        let script = Script::with(Vec::new());
        conn(&script)
            .send_frame(Opcode::Text, b"{}")
            .expect("the frame went out");
        assert_eq!(script.writes(), vec![vec![0x81, 0x02], b"{}".to_vec()]);
    }

    #[test]
    fn an_empty_frame_writes_its_header_alone() {
        let script = Script::with(Vec::new());
        conn(&script)
            .send_frame(Opcode::Close, &[])
            .expect("the close went out");
        assert_eq!(script.writes(), vec![vec![0x88, 0x00]]);
    }

    #[test]
    fn a_payload_needing_a_64_bit_length_fails_without_writing_a_byte() {
        let script = Script::with(Vec::new());
        let huge = vec![0u8; usize::from(u16::MAX) + 1];
        assert_eq!(
            conn(&script).send_frame(Opcode::Text, &huge),
            Err(WireError::Failed)
        );
        assert!(
            script.writes().is_empty(),
            "a refused frame must not leave a header on the wire"
        );
    }

    #[test]
    fn a_second_sender_cannot_enter_while_a_frame_is_half_written() {
        let script = Script::with(Vec::new());
        let (announce, announced) = std::sync::mpsc::channel();
        *script.hold_first_write.lock().expect("the hold") = Some(announced);
        let shared = Arc::new(conn(&script));

        let intruder = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let _ = announce.send(());
                shared.send_frame(Opcode::Text, b"bbb").expect("the frame");
            })
        };
        shared.send_frame(Opcode::Text, b"aaa").expect("the frame");
        intruder.join().expect("the second sender");

        assert_eq!(
            script.most_inside.load(Ordering::SeqCst),
            1,
            "a second sender wrote while the first frame was still unfinished"
        );
        let writes = script.writes();
        assert_eq!(writes.len(), 4);
        for pair in writes.chunks(2) {
            assert_eq!(pair[0], vec![0x81, 0x03], "a payload landed where a header belongs");
            assert!(pair[1].iter().all(|byte| *byte == pair[1][0]));
        }
    }

    #[test]
    fn the_wire_is_finished_once_when_its_last_owner_goes() {
        let script = Script::with(Vec::new());
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
        let script = Script::with(Vec::new());
        conn(&script).wake_reader();
        assert_eq!(script.woken.load(Ordering::Relaxed), 1);
    }
}
