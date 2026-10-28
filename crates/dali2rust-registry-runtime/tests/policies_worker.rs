mod support;

use dali2rust_bus::BusId;
use dali2rust_contracts::msg::{
    DeliveryStatus, DeviceType, ErrorCode, PoliciesUpdateCommand, PolicyException,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::PoliciesReadPort;

use support::{assert_exec_failed_msg, publish_cmd, recv_confirm_for, seed_physical_via_discovery, spawn_registry_stack};

const KNOWN_SHORT: u8 = 2;
const ABSENT_SHORT: u8 = 9;

fn excepting(correlation_id: u64, shorts: &[u8]) -> dali2rust_contracts::msg::CommandEnvelope {
    let exceptions = shorts.iter().map(|short| PolicyException { adapter_id: 0, short_address: *short });
    let body = PoliciesUpdateCommand {
        patch_mask: PoliciesUpdateCommand::PATCH_EXCEPTIONS,
        system_failure_level: PoliciesUpdateCommand::UNMANAGED,
        power_on_level: PoliciesUpdateCommand::UNMANAGED,
        apply_on_discovery: false,
        exceptions: exceptions.collect(),
    };
    dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, correlation_id, BusId::default().0, None, body)
}

#[test]
fn the_writer_refuses_an_exception_naming_no_record_and_stores_nothing() {
    let stack = spawn_registry_stack(1, 8);
    seed_physical_via_discovery(&stack.publisher, 1, KNOWN_SHORT, DeviceType::Dt6Led, &stack.store);

    publish_cmd(&stack.publisher, excepting(2, &[KNOWN_SHORT, ABSENT_SHORT]));
    let refused = recv_confirm_for(&stack.conf_rx, 2);
    assert_exec_failed_msg(&refused, "unknown_device");
    let code = refused.confirmation.error.as_ref().map(|error| error.code);
    assert_eq!(code, Some(ErrorCode::UnknownDevice), "the handler and the writer name one cause");
    assert!(stack.store.policies_view().exceptions.is_empty(), "a refused list stores no entry");

    publish_cmd(&stack.publisher, excepting(3, &[KNOWN_SHORT]));
    assert_eq!(recv_confirm_for(&stack.conf_rx, 3).status, DeliveryStatus::Ok);
    let stored: Vec<u8> = stack.store.policies_view().exceptions.iter().map(|e| e.short_address).collect();
    assert_eq!(stored, vec![KNOWN_SHORT]);
}
