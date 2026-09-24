#![allow(dead_code, reason = "Each `tests/*.rs` is its own binary and compiles this whole module, so a helper one binary does not call is \"never used\" from that binary's point of view — the same reason `dali2rust-registry-runtime`'s support module carries this, and the same pressure that produced the local copies in the first place")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusFrame, BusId, BusPublisher, BusSubscriberRx};
use dali2rust_contracts::msg::{
    DaliAttributeGroup, DaliReadAttributesCommand, DaliSetTargetStateCommand, DaliTargetScope,
    LightSetpoint, MemoryBankReadPreset, Origin, PowerState,
};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
};
use dali2rust_domain::dali::pres::command::DaliCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_platform::dali::WireActivity;
use dali2rust_registry_runtime::RegistryStore;

pub const SHORT: u8 = 5;
pub const LEVEL: u8 = 220;
pub const EVENT_BUDGET: Duration = Duration::from_secs(5);

pub fn standard_frame(command: StandardCommand) -> u16 {
    DaliCommand::Standard {
        address: DaliAddress::short(SHORT).expect("short"),
        command,
    }
    .to_forward_frame()
    .raw()
}

pub fn dapc_frame() -> u16 {
    standard_frame(StandardCommand::DirectArcPower { level: LEVEL })
}

pub fn read_frame(correlation_id: u64, origin: Origin, groups: DaliAttributeGroup) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        Some(origin),
        DaliReadAttributesCommand {
            registry_adapter_id: 0,
            short_address: SHORT,
            attribute_groups_mask: groups.mask_bit(),
            memory_banks: MemoryBankReadPreset::None,
        },
    ))
}

pub fn api_read_frame(correlation_id: u64) -> BusFrame {
    read_frame(correlation_id, Origin::Api, DaliAttributeGroup::RuntimeStatus)
}

pub fn poller_read_frame(correlation_id: u64) -> BusFrame {
    read_frame(
        correlation_id,
        Origin::Poller,
        DaliAttributeGroup::RuntimeStatus,
    )
}

pub fn dt8_read_frame(correlation_id: u64) -> BusFrame {
    read_frame(correlation_id, Origin::Api, DaliAttributeGroup::Dt8Color)
}

pub fn target_state_frame(
    correlation_id: u64,
    origin: Origin,
    scope: DaliTargetScope,
    short_address: u8,
    level: u8,
) -> BusFrame {
    BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        Some(origin),
        DaliSetTargetStateCommand {
            registry_adapter_id: 0,
            scope,
            short_address,
            virtual_lamp_id: 0,
            group_id: 0,
            setpoint: LightSetpoint {
                power: PowerState::On,
                level,
                color: None,
            },
        },
    ))
}

pub fn interactive_target_state_frame(correlation_id: u64) -> BusFrame {
    target_state_frame(
        correlation_id,
        Origin::Api,
        DaliTargetScope::Short,
        SHORT,
        LEVEL,
    )
}

pub fn spawn_mock_worker(
    worker_cmd: BusSubscriberRx,
    transport: &Arc<Mutex<MockDaliTransport>>,
    publisher: &BusPublisher,
    counters: &Arc<DaliWorkerCounters>,
    interactive: Arc<WireActivity>,
) -> std::thread::JoinHandle<()> {
    spawn_dali_worker(
        worker_cmd,
        DaliController::new(Arc::clone(transport), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::new(RegistryStore::with_adapter_count(1)),
        publisher.clone(),
        BusId::default(),
        Arc::clone(counters),
        interactive,
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    )
}
