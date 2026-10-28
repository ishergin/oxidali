use dali2rust_platform::answer_table::{AnswerEntry, AnswerTableImage, NO_SHORT_ADDRESS};

use crate::dali::dev103::address::MAX_SHORT_ADDRESS;
use crate::dali::dev103::command::Device103Command;

pub const ANSWER_YES: u8 = 0xFF;

// IEC 62386-103 §11.6.16
#[must_use]
pub fn answer_table(application_active: bool, device_short_address: Option<u8>) -> AnswerTableImage {
    let mut image = AnswerTableImage::SILENT;
    image.short_address = device_short_address
        .filter(|address| *address <= MAX_SHORT_ADDRESS)
        .unwrap_or(NO_SHORT_ADDRESS);
    let enabled = if application_active {
        AnswerEntry::by_lease(Some(ANSWER_YES), None)
    } else {
        AnswerEntry::SILENT
    };
    let opcode = Device103Command::QueryApplicationControlEnabled.metadata().opcode;
    image.set_device_query(opcode, enabled);
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dali::dev103::address::{Device103Address, InstanceAddress};
    use crate::dali::dev103::command::Special103Command;
    use crate::dali::dev103::frame::ForwardFrame24;
    use dali2rust_dali_phy::{AnswerTableCell, TableLookup};
    use dali2rust_platform::answer_table::{
        COMPARE_ENTRY, QUERY_SHORT_ADDRESS_ENTRY, VERIFY_SHORT_ADDRESS_ENTRY,
    };

    const NOW: u32 = 1_000;

    fn qace(address: Device103Address) -> [u8; 3] {
        let opcode = Device103Command::QueryApplicationControlEnabled.metadata().opcode;
        ForwardFrame24::command(address, InstanceAddress::Device, opcode).as_bytes()
    }

    fn answer(active: bool, short: Option<u8>, frame: [u8; 3]) -> TableLookup {
        let cell = AnswerTableCell::new();
        cell.publish(&answer_table(active, short), 0);
        cell.set_lease_until_tick(NOW + 1);
        cell.lookup(frame, 0, NOW)
    }

    fn yes(active: bool, short: Option<u8>, address: Device103Address) -> bool {
        answer(active, short, qace(address)) == TableLookup::Answer(ANSWER_YES)
    }

    #[test]
    fn a_passive_controller_answers_nothing_because_no_is_silence() {
        for short in [Some(7), None] {
            assert!(!yes(false, short, Device103Address::Broadcast));
            assert!(!yes(false, short, Device103Address::BroadcastUnaddressed));
        }
        assert!(!yes(false, Some(7), Device103Address::Short(7)));
    }

    #[test]
    fn an_active_controller_answers_the_broadcast_form() {
        assert!(yes(true, Some(7), Device103Address::Broadcast));
        assert!(yes(true, None, Device103Address::Broadcast));
    }

    #[test]
    fn a_configured_short_address_is_answered_and_the_unaddressed_form_is_not() {
        assert!(yes(true, Some(7), Device103Address::Short(7)));
        assert!(!yes(true, Some(7), Device103Address::Short(8)));
        assert!(!yes(true, Some(7), Device103Address::BroadcastUnaddressed));
    }

    #[test]
    fn an_unset_short_address_answers_the_unaddressed_broadcast_instead() {
        assert!(yes(true, None, Device103Address::BroadcastUnaddressed));
    }

    #[test]
    fn an_out_of_range_short_address_is_read_as_unset_never_clamped() {
        assert!(yes(true, Some(200), Device103Address::BroadcastUnaddressed));
        assert!(!yes(true, Some(200), Device103Address::Short(63)));
    }

    #[test]
    fn short_address_zero_is_a_real_address_not_an_absent_one() {
        assert!(yes(true, Some(0), Device103Address::Short(0)));
        assert!(!yes(true, Some(0), Device103Address::BroadcastUnaddressed));
    }

    #[test]
    fn the_answer_lives_only_as_long_as_the_lease() {
        let cell = AnswerTableCell::new();
        cell.publish(&answer_table(true, Some(7)), 0);
        cell.set_lease_until_tick(NOW);
        assert_eq!(cell.lookup(qace(Device103Address::Broadcast), 0, NOW), TableLookup::LeaseLapsed);
    }

    #[test]
    fn each_special_query_reaches_its_own_entry() {
        let mut image = answer_table(true, Some(7));
        image.entries[COMPARE_ENTRY] = AnswerEntry::always(0x11);
        image.entries[VERIFY_SHORT_ADDRESS_ENTRY] = AnswerEntry::always(0x22);
        image.entries[QUERY_SHORT_ADDRESS_ENTRY] = AnswerEntry::always(0x33);
        let cell = AnswerTableCell::new();
        cell.publish(&image, 0);
        let lookup = |command: Special103Command, data: u8| {
            cell.lookup(command.frame(data).as_bytes(), 0, NOW)
        };
        assert_eq!(lookup(Special103Command::Compare, 0), TableLookup::Answer(0x11));
        assert_eq!(lookup(Special103Command::VerifyShortAddress, 7), TableLookup::Answer(0x22));
        assert_eq!(lookup(Special103Command::QueryShortAddress, 0), TableLookup::Answer(0x33));
        assert_eq!(lookup(Special103Command::Withdraw, 0), TableLookup::NotAddressed);
    }

    #[test]
    fn no_other_device_query_is_answered_yet() {
        let status = Device103Command::QueryDeviceStatus.metadata().opcode;
        let frame = ForwardFrame24::command(Device103Address::Short(7), InstanceAddress::Device, status);
        assert_eq!(answer(true, Some(7), frame.as_bytes()), TableLookup::Silent);
    }
}
