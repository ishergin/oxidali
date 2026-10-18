mod support;

use dali2rust_bus::{BusFrame, BusId};
use dali2rust_contracts::msg::{
    BusCommandPayload, Dali103InstanceConfigureCommand, DaliAddressingCommand,
    InstancePatchField, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::dali::dev103::EventScheme;
use support::{assert_a_lost_fact_fails_the_operation, DemoBusOperation};

const OPERATION_CORRELATION: u64 = 6_000;
const DEMO_GEAR_SHORT: u8 = 3;
const FREE_SHORT: u8 = 40;
const DEMO_PANEL_SHORT: u8 = 0;
const DEMO_INSTANCE: u8 = 0;

fn operation_frame(payload: impl Into<BusCommandPayload>) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        OPERATION_CORRELATION,
        BusId::default().0,
        Some(Origin::Api),
        payload.into(),
    ))
}

fn address_change() -> BusFrame {
    operation_frame(DaliAddressingCommand {
        registry_adapter_id: 0,
        short_address: DEMO_GEAR_SHORT,
        new_short_address: FREE_SHORT,
        verify_after_program: true,
        operation_key: Default::default(),
    })
}

fn event_scheme_patch() -> BusFrame {
    operation_frame(Dali103InstanceConfigureCommand {
        registry_adapter_id: 0,
        short_address: DEMO_PANEL_SHORT,
        instance_number: DEMO_INSTANCE,
        patch_mask: InstancePatchField::EventScheme.bit(),
        event_scheme: EventScheme::Device.code(),
        event_filter: [0; 3],
        event_priority: 0,
        instance_groups: [None; 3],
        timer_multipliers: [None; 4],
        instance_enabled: true,
    })
}

#[test]
fn a_lost_addressing_fact_fails_the_address_change() {
    assert_a_lost_fact_fails_the_operation(&DemoBusOperation {
        frame: address_change,
        correlation_id: OPERATION_CORRELATION,
        announces_start: true,
        lost: "addressing_completed_event_lost",
    });
}

#[test]
fn a_lost_configured_fact_fails_the_instance_configuration() {
    assert_a_lost_fact_fails_the_operation(&DemoBusOperation {
        frame: event_scheme_patch,
        correlation_id: OPERATION_CORRELATION,
        announces_start: false,
        lost: "instance_configured_event_lost",
    });
}
