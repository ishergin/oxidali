use super::connection::{ReadError, ReadOutcome, WsConnection, WsWire};
use super::frame::{FrameError, Opcode};
use super::hub::ClientId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderExit {
    PeerClosed,
    SocketClosed,
    Desync,
    Protocol(FrameError),
    BadUtf8,
    Oversize,
    Failed,
    NotServed,
}

pub trait ClientFrames: Send + Sync {
    fn text(&self, id: ClientId, text: &str);
    fn serves(&self, id: ClientId) -> bool;
    fn forget(&self, id: ClientId);
}

enum Step {
    Carry,
    Stop(ReaderExit),
}

pub fn serve_client_frames<W: WsWire>(
    frames: &dyn ClientFrames,
    id: ClientId,
    conn: &WsConnection<W>,
    buffer: &mut [u8],
) -> ReaderExit {
    let exit = read_until_done(frames, id, conn, buffer);
    frames.forget(id);
    exit
}

fn read_until_done<W: WsWire>(
    frames: &dyn ClientFrames,
    id: ClientId,
    conn: &WsConnection<W>,
    buffer: &mut [u8],
) -> ReaderExit {
    loop {
        if !frames.serves(id) {
            return ReaderExit::NotServed;
        }
        let step = match conn.read_frame(buffer) {
            Ok(ReadOutcome::Idle) => Step::Carry,
            Ok(ReadOutcome::Frame {
                opcode,
                payload_len,
            }) => dispatch(frames, id, conn, opcode, &buffer[..payload_len]),
            Err(err) => stop_for(conn, err),
        };
        if let Step::Stop(exit) = step {
            return exit;
        }
    }
}

fn dispatch<W: WsWire>(
    frames: &dyn ClientFrames,
    id: ClientId,
    conn: &WsConnection<W>,
    opcode: Opcode,
    payload: &[u8],
) -> Step {
    match opcode {
        Opcode::Text => forward_text(frames, id, conn, payload),
        Opcode::Ping => match conn.send_frame(Opcode::Pong, payload) {
            Ok(()) => Step::Carry,
            Err(_) => Step::Stop(ReaderExit::Failed),
        },
        Opcode::Close => close_then(conn, ReaderExit::PeerClosed),
        Opcode::Binary | Opcode::Pong => Step::Carry,
        Opcode::Continuation => close_then(conn, ReaderExit::Protocol(FrameError::Fragmented)),
    }
}

fn forward_text<W: WsWire>(
    frames: &dyn ClientFrames,
    id: ClientId,
    conn: &WsConnection<W>,
    payload: &[u8],
) -> Step {
    match core::str::from_utf8(payload) {
        Ok(text) => {
            frames.text(id, text);
            Step::Carry
        }
        Err(_) => close_then(conn, ReaderExit::BadUtf8),
    }
}

fn stop_for<W: WsWire>(conn: &WsConnection<W>, err: ReadError) -> Step {
    match err {
        ReadError::Closed => Step::Stop(ReaderExit::SocketClosed),
        ReadError::Failed => Step::Stop(ReaderExit::Failed),
        ReadError::Desync => close_then(conn, ReaderExit::Desync),
        ReadError::Oversize => close_then(conn, ReaderExit::Oversize),
        ReadError::Protocol(cause) => close_then(conn, ReaderExit::Protocol(cause)),
    }
}

fn close_then<W: WsWire>(conn: &WsConnection<W>, exit: ReaderExit) -> Step {
    let _ = conn.send_frame(Opcode::Close, &[]);
    Step::Stop(exit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::connection::WireError;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    const BODY: usize = 512;

    const CLIENT: ClientId = 7;

    struct Tape {
        reads: Mutex<Vec<Result<Vec<u8>, WireError>>>,
        writes: Mutex<Vec<Vec<u8>>>,
    }

    impl Tape {
        fn new(reads: Vec<Result<Vec<u8>, WireError>>) -> Self {
            Self {
                reads: Mutex::new(reads),
                writes: Mutex::new(Vec::new()),
            }
        }
    }

    impl WsWire for &'static Tape {
        fn read(&self, buf: &mut [u8]) -> Result<usize, WireError> {
            let mut reads = self.reads.lock().expect("the tape");
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

        fn write_all(&self, buf: &[u8]) -> Result<(), WireError> {
            self.writes.lock().expect("the writes").push(buf.to_vec());
            Ok(())
        }

        fn wake_reader(&self) {}

        fn finish(&self) {}
    }

    #[derive(Default)]
    struct Frames {
        texts: Mutex<Vec<String>>,
        forgotten: AtomicUsize,
        served: AtomicBool,
    }

    impl Frames {
        fn serving() -> Self {
            Self {
                served: AtomicBool::new(true),
                ..Self::default()
            }
        }

        fn texts(&self) -> Vec<String> {
            self.texts.lock().expect("the texts").clone()
        }
    }

    impl ClientFrames for Frames {
        fn text(&self, id: ClientId, text: &str) {
            assert_eq!(id, CLIENT, "a frame was forwarded under the wrong client");
            self.texts.lock().expect("the texts").push(text.to_owned());
        }

        fn serves(&self, _id: ClientId) -> bool {
            self.served.load(Ordering::Relaxed)
        }

        fn forget(&self, _id: ClientId) {
            self.forgotten.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn masked(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let key = [0x11, 0x22, 0x33, 0x44];
        let mut frame = vec![0x80 | opcode, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&key);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ key[index % 4]),
        );
        frame
    }

    fn run(reads: Vec<Result<Vec<u8>, WireError>>) -> (ReaderExit, &'static Tape, Frames) {
        // SAFETY: test-only leak; the tape must outlive the connection that borrows it.
        let tape: &'static Tape = Box::leak(Box::new(Tape::new(reads)));
        let frames = Frames::serving();
        let conn = WsConnection::new(tape);
        let mut body = [0u8; BODY];
        let exit = serve_client_frames(&frames, CLIENT, &conn, &mut body);
        (exit, tape, frames)
    }

    fn writes(tape: &Tape) -> Vec<Vec<u8>> {
        tape.writes.lock().expect("the writes").clone()
    }

    #[test]
    fn text_frames_reach_the_hub_in_order_until_the_socket_closes() {
        let (exit, _, frames) = run(vec![
            Ok(masked(0x1, b"one")),
            Ok(masked(0x1, b"two")),
            Ok(Vec::new()),
        ]);
        assert_eq!(exit, ReaderExit::SocketClosed);
        assert_eq!(frames.texts(), vec!["one".to_owned(), "two".to_owned()]);
        assert_eq!(frames.forgotten.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn an_idle_turn_does_not_end_the_loop() {
        let (exit, _, frames) = run(vec![
            Err(WireError::Timeout),
            Err(WireError::Timeout),
            Ok(masked(0x1, b"late")),
            Ok(Vec::new()),
        ]);
        assert_eq!(exit, ReaderExit::SocketClosed);
        assert_eq!(frames.texts(), vec!["late".to_owned()]);
    }

    #[test]
    fn a_ping_is_answered_with_a_pong_carrying_the_same_payload() {
        let (exit, tape, _) = run(vec![Ok(masked(0x9, b"ab")), Ok(Vec::new())]);
        assert_eq!(exit, ReaderExit::SocketClosed);
        assert_eq!(
            writes(tape),
            vec![vec![0x8a, 0x02], b"ab".to_vec()],
            "a ping must come back as an unmasked pong of the same body"
        );
    }

    #[test]
    fn a_peer_close_is_answered_and_ends_the_loop() {
        let (exit, tape, frames) = run(vec![Ok(masked(0x8, &[])), Ok(masked(0x1, b"after"))]);
        assert_eq!(exit, ReaderExit::PeerClosed);
        assert_eq!(writes(tape), vec![vec![0x88, 0x00]]);
        assert!(
            frames.texts().is_empty(),
            "a frame after the close was still forwarded"
        );
    }

    #[test]
    fn a_protocol_violation_closes_the_connection_rather_than_being_skipped() {
        let (exit, tape, frames) = run(vec![Ok(vec![0x81, 0x03, b'a', b'b', b'c'])]);
        assert_eq!(exit, ReaderExit::Protocol(FrameError::Unmasked));
        assert_eq!(writes(tape), vec![vec![0x88, 0x00]]);
        assert!(frames.texts().is_empty());
    }

    #[test]
    fn a_fragmented_text_frame_closes_the_connection() {
        let mut frame = masked(0x1, b"half");
        frame[0] &= 0x7f;
        let (exit, tape, _) = run(vec![Ok(frame)]);
        assert_eq!(exit, ReaderExit::Protocol(FrameError::Fragmented));
        assert_eq!(writes(tape), vec![vec![0x88, 0x00]]);
    }

    #[test]
    fn a_text_frame_that_is_not_utf8_closes_the_connection() {
        let (exit, tape, frames) = run(vec![Ok(masked(0x1, &[0xff, 0xfe]))]);
        assert_eq!(exit, ReaderExit::BadUtf8);
        assert_eq!(writes(tape), vec![vec![0x88, 0x00]]);
        assert!(frames.texts().is_empty());
    }

    #[test]
    fn a_desync_closes_the_connection() {
        let (exit, tape, _) = run(vec![Ok(vec![0x81]), Err(WireError::Timeout)]);
        assert_eq!(exit, ReaderExit::Desync);
        assert_eq!(writes(tape), vec![vec![0x88, 0x00]]);
    }

    #[test]
    fn binary_and_pong_frames_are_ignored_without_ending_the_loop() {
        let (exit, tape, frames) = run(vec![
            Ok(masked(0x2, b"raw")),
            Ok(masked(0xa, b"pong")),
            Ok(masked(0x1, b"still here")),
            Ok(Vec::new()),
        ]);
        assert_eq!(exit, ReaderExit::SocketClosed);
        assert_eq!(frames.texts(), vec!["still here".to_owned()]);
        assert!(
            writes(tape).is_empty(),
            "an ignored frame must not answer anything"
        );
    }

    #[test]
    fn a_client_the_hub_no_longer_serves_ends_the_loop_before_reading() {
        // SAFETY: test-only leak; the tape must outlive the connection that borrows it.
        let tape: &'static Tape = Box::leak(Box::new(Tape::new(vec![Ok(masked(0x1, b"x"))])));
        let frames = Frames::default();
        let conn = WsConnection::new(tape);
        let mut body = [0u8; BODY];
        let exit = serve_client_frames(&frames, CLIENT, &conn, &mut body);
        assert_eq!(exit, ReaderExit::NotServed);
        assert!(frames.texts().is_empty(), "a dropped client was still served");
        assert_eq!(frames.forgotten.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn the_client_is_forgotten_on_every_way_out() {
        for reads in [
            vec![Ok(Vec::new())],
            vec![Ok(masked(0x8, &[]))],
            vec![Ok(vec![0x81, 0x03, b'a', b'b', b'c'])],
            vec![Err(WireError::Failed)],
        ] {
            let (_, _, frames) = run(reads);
            assert_eq!(
                frames.forgotten.load(Ordering::Relaxed),
                1,
                "an exit left the client registered"
            );
        }
    }
}
