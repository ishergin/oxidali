use dali2rust_contracts::msg::GroupMembershipAction;
use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;

use crate::runtime::executor::attributes::read_group_membership_mask;
use crate::runtime::executor::helpers::{
    dali_short_address, program_with_verify_repair, send_standard, SemanticDaliError,
};

pub fn program_group_membership(
    controller: &mut impl DaliApplicationController,
    short_address: u8,
    group_id: u8,
    action: GroupMembershipAction,
) -> Result<Option<u16>, SemanticDaliError> {
    let address = dali_short_address(short_address)?;
    program_with_verify_repair(
        || drive_membership_program(controller, address, short_address, group_id, action),
        |readback| membership_confirms(readback, group_id, action),
    )
}

fn drive_membership_program(
    controller: &mut impl DaliApplicationController,
    address: DaliAddress,
    short_address: u8,
    group_id: u8,
    action: GroupMembershipAction,
) -> Result<Option<u16>, SemanticDaliError> {
    let command = match action {
        GroupMembershipAction::Add => StandardCommand::AddToGroup { group: group_id },
        GroupMembershipAction::Remove => StandardCommand::RemoveFromGroup { group: group_id },
    };
    send_standard(controller, address, command)?;
    Ok(read_group_membership_mask(controller, short_address).unwrap_or(None))
}

fn membership_confirms(readback: Option<u16>, group_id: u8, action: GroupMembershipAction) -> bool {
    let Some(mask) = readback else {
        return false;
    };
    let bit_set = (mask >> group_id) & 1 == 1;
    bit_set == matches!(action, GroupMembershipAction::Add)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::executor::test_helpers::shared::{
        assert_script_consumed, setup_controller, short_address,
    };
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_domain::dali::commands::DaliCommand;

    const SHORT: u8 = 4;
    const GROUP: u8 = 3;
    const GROUP_BIT: u16 = 1 << GROUP;

    fn standard_frame(command: StandardCommand) -> u16 {
        DaliCommand::Standard {
            address: short_address(SHORT),
            command,
        }
        .to_forward_frame()
        .raw()
    }

    fn expect_add_drive(mock: &MockDaliTransport, lo: Option<u8>, hi: Option<u8>) {
        let add = standard_frame(StandardCommand::AddToGroup { group: GROUP });
        mock.expect_forward_frame(add);
        mock.expect_forward_frame(add);
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QueryGroups0To7),
            lo,
        );
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QueryGroups8To15),
            hi,
        );
    }

    #[test]
    fn confirmed_first_readback_needs_no_repair() {
        let mock = MockDaliTransport::new();
        expect_add_drive(&mock, Some(GROUP_BIT as u8), Some(0));

        let (transport, mut controller) = setup_controller(mock);
        let mask =
            program_group_membership(&mut controller, SHORT, GROUP, GroupMembershipAction::Add)
                .expect("membership add");
        assert_eq!(mask, Some(GROUP_BIT));
        assert_script_consumed(&transport);
    }

    #[test]
    fn missing_bit_redrives_the_config_pair_once() {
        let mock = MockDaliTransport::new();
        expect_add_drive(&mock, Some(0), Some(0));
        expect_add_drive(&mock, Some(GROUP_BIT as u8), Some(0));

        let (transport, mut controller) = setup_controller(mock);
        let mask =
            program_group_membership(&mut controller, SHORT, GROUP, GroupMembershipAction::Add)
                .expect("membership add repaired");
        assert_eq!(mask, Some(GROUP_BIT));
        assert_script_consumed(&transport);
    }

    #[test]
    fn unanswered_repair_readback_keeps_last_answered_mask() {
        let mock = MockDaliTransport::new();
        expect_add_drive(&mock, Some(0), Some(0));
        expect_add_drive(&mock, None, None);
        expect_add_drive(&mock, None, None);

        let (transport, mut controller) = setup_controller(mock);
        let mask =
            program_group_membership(&mut controller, SHORT, GROUP, GroupMembershipAction::Add)
                .expect("membership add with silent repair readback");
        assert_eq!(mask, Some(0));
        assert_script_consumed(&transport);
    }

    #[test]
    fn remove_confirms_on_cleared_bit() {
        let mock = MockDaliTransport::new();
        let remove = standard_frame(StandardCommand::RemoveFromGroup { group: GROUP });
        mock.expect_forward_frame(remove);
        mock.expect_forward_frame(remove);
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QueryGroups0To7),
            Some(0),
        );
        mock.expect_forward_frame_with_backward(
            standard_frame(StandardCommand::QueryGroups8To15),
            Some(0),
        );

        let (transport, mut controller) = setup_controller(mock);
        let mask =
            program_group_membership(&mut controller, SHORT, GROUP, GroupMembershipAction::Remove)
                .expect("membership remove");
        assert_eq!(mask, Some(0));
        assert_script_consumed(&transport);
    }
}
