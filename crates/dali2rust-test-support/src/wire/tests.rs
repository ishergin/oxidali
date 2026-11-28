use std::convert::Infallible;

use super::*;

#[derive(Default)]
struct Listener {
    sent: Vec<u16>,
    heard: Vec<u16>,
}

impl DaliTransport for Listener {
    type Error = Infallible;

    fn send_forward_frame(&mut self, frame: u16) -> Result<(), Infallible> {
        self.sent.push(frame);
        Ok(())
    }

    fn receive_backward_frame(&mut self) -> Result<Option<u8>, Infallible> {
        Ok(None)
    }

    fn is_bus_idle(&self) -> Result<bool, Infallible> {
        Ok(true)
    }
}

fn hear(ear: &mut Listener, frame: u16) -> bool {
    ear.heard.push(frame);
    true
}

const GROUP_5_UP: u16 = 0x8B01;
const BROADCAST_OFF: u16 = 0xFF00;

#[test]
fn a_frame_one_tap_sends_is_heard_by_every_other_tap_and_never_by_itself() {
    let wire = SharedWire::new(hear);
    let (mut first, second, third) = (wire.tap(Listener::default()), wire.tap(Listener::default()), wire.tap(Listener::default()));
    first.exchange_frame(GROUP_5_UP, false).ok();
    assert_eq!(first.transport().lock().map(|ear| (ear.sent.clone(), ear.heard.clone())).ok(), Some((vec![GROUP_5_UP], vec![])));
    for other in [&second, &third] {
        assert_eq!(other.transport().lock().map(|ear| ear.heard.clone()).ok(), Some(vec![GROUP_5_UP]));
    }
    assert_eq!(wire.sent_by(first.index()), [GROUP_5_UP]);
}

#[test]
fn a_foreign_master_is_heard_by_every_tap_and_sent_by_none() {
    let wire = SharedWire::new(hear);
    let taps = [wire.tap(Listener::default()), wire.tap(Listener::default())];
    assert_eq!(wire.foreign(BROADCAST_OFF), taps.len());
    for tap in &taps {
        assert_eq!(tap.transport().lock().map(|ear| (ear.sent.len(), ear.heard.clone())).ok(), Some((0, vec![BROADCAST_OFF])));
    }
    assert_eq!(wire.frames(), [WireFrame { speaker: Speaker::Foreign, frame: BROADCAST_OFF }]);
}
