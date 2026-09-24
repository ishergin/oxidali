use crate::dali::dev103::address::{Device103Address, InstanceAddress, MAX_SHORT_ADDRESS};
use crate::dali::dev103::command::Device103Command;
use crate::dali::dev103::frame::ForwardFrame24;

pub const MAX_ARBITRATION_SLOTS: usize = 2;

pub const ANSWER_YES: u8 = 0xFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReflexSlot {
    pub frame: [u8; 3],
    pub answer: u8,
}

// IEC 62386-103 §11.6.16
#[must_use]
pub fn arbitration_slots(
    application_active: bool,
    device_short_address: Option<u8>,
) -> heapless_slots::Slots {
    let mut out = heapless_slots::Slots::new();
    if !application_active {
        return out;
    }
    out.push(slot_for(Device103Address::Broadcast));
    match short_address(device_short_address) {
        Some(address) => out.push(slot_for(Device103Address::Short(address))),
        None => out.push(slot_for(Device103Address::BroadcastUnaddressed)),
    }
    out
}

fn short_address(configured: Option<u8>) -> Option<u8> {
    configured.filter(|a| *a <= MAX_SHORT_ADDRESS)
}

fn slot_for(address: Device103Address) -> ReflexSlot {
    let frame = ForwardFrame24::command(
        address,
        InstanceAddress::Device,
        Device103Command::QueryApplicationControlEnabled
            .metadata()
            .opcode,
    );
    ReflexSlot {
        frame: frame.as_bytes(),
        answer: ANSWER_YES,
    }
}

pub mod heapless_slots {
    use super::{ReflexSlot, MAX_ARBITRATION_SLOTS};

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct Slots {
        items: [Option<ReflexSlot>; MAX_ARBITRATION_SLOTS],
        len: usize,
    }

    impl Slots {
        #[must_use]
        pub const fn new() -> Self {
            Self {
                items: [None; MAX_ARBITRATION_SLOTS],
                len: 0,
            }
        }

        pub(super) fn push(&mut self, slot: ReflexSlot) {
            if let Some(cell) = self.items.get_mut(self.len) {
                *cell = Some(slot);
                self.len = self.len.saturating_add(1);
            }
        }

        #[must_use]
        pub fn len(&self) -> usize {
            self.len
        }

        #[must_use]
        pub fn is_empty(&self) -> bool {
            self.len == 0
        }

        pub fn iter(&self) -> impl Iterator<Item = ReflexSlot> + '_ {
            self.items.iter().take(self.len).filter_map(|s| *s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUERY_OPCODE: u8 = 0x3D;

    fn frames(active: bool, short: Option<u8>) -> Vec<[u8; 3]> {
        arbitration_slots(active, short).iter().map(|s| s.frame).collect()
    }

    #[test]
    fn a_passive_controller_publishes_nothing_because_no_is_silence() {
        assert!(arbitration_slots(false, Some(7)).is_empty());
        assert!(arbitration_slots(false, None).is_empty());
    }

    #[test]
    fn two_standbys_cannot_answer_each_other() {
        assert!(arbitration_slots(false, Some(1)).is_empty());
        assert!(arbitration_slots(false, Some(2)).is_empty());
    }

    #[test]
    fn an_active_controller_answers_the_broadcast_form() {
        let f = frames(true, Some(7));
        assert!(f.contains(&[0xFF, 0xFE, QUERY_OPCODE]));
    }

    #[test]
    fn a_configured_short_address_is_answered_and_the_unaddressed_form_is_not() {
        let f = frames(true, Some(7));
        assert!(f.contains(&[0x0F, 0xFE, QUERY_OPCODE]));
        assert!(!f.contains(&[0xFD, 0xFE, QUERY_OPCODE]));
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn an_unset_short_address_answers_the_unaddressed_broadcast_instead() {
        let f = frames(true, None);
        assert!(f.contains(&[0xFD, 0xFE, QUERY_OPCODE]));
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn an_out_of_range_short_address_is_read_as_unset_never_clamped() {
        let f = frames(true, Some(200));
        assert!(f.contains(&[0xFD, 0xFE, QUERY_OPCODE]));
        assert!(!f.contains(&[(63 << 1) | 1, 0xFE, QUERY_OPCODE]));
    }

    #[test]
    fn every_published_frame_is_a_command_not_an_event() {
        for slot in arbitration_slots(true, Some(31)).iter() {
            assert!(ForwardFrame24::from_bytes(slot.frame).is_command());
        }
    }

    #[test]
    fn the_answer_is_mask() {
        for slot in arbitration_slots(true, None).iter() {
            assert_eq!(slot.answer, ANSWER_YES);
        }
    }

    #[test]
    fn the_published_opcode_is_the_one_351_asks_with() {
        for slot in arbitration_slots(true, Some(0)).iter() {
            assert_eq!(slot.frame[2], QUERY_OPCODE);
            assert_eq!(slot.frame[1], InstanceAddress::Device.encode());
        }
    }

    #[test]
    fn short_address_zero_is_a_real_address_not_an_absent_one() {
        let f = frames(true, Some(0));
        assert!(f.contains(&[0x01, 0xFE, QUERY_OPCODE]));
        assert!(!f.contains(&[0xFD, 0xFE, QUERY_OPCODE]));
    }
}
