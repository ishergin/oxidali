use std::sync::Arc;
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_contracts::msg::{BusEventPayload, DeliveryStatus};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
};
use dali2rust_registry_runtime::RegistryStore;

#[test]
fn command_envelope_produces_confirmation_and_wire_event() {
    let (_host, publisher, (worker_cmd, conf_obs, ev_obs)) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            (
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_confirmations(16),
                reg.subscribe_events(16, dali2rust_contracts::msg::EVENT_VARIANT_NAMES),
            )
        },
    );
    let transport = Arc::new(std::sync::Mutex::new(MockDaliTransport::new()));
    transport.lock().unwrap().set_persistent_response(100);
    let controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
    let read_port = Arc::new(RegistryStore::with_adapter_count(1));
    let _worker = spawn_dali_worker(
        worker_cmd,
        controller,
        DaliRuntimeConfig::default(),
        read_port,
        publisher.clone(),
        BusId::default(),
        Arc::new(DaliWorkerCounters::default()),
        Arc::new(dali2rust_platform::dali::WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    );
    let frame = BusFrame::command(dali2rust_contracts::bus::command_envelope(0, 1, BusId::default().0, None, dali2rust_contracts::msg::DaliCommandPayload { wire_address: 0, command: 144, repeat_count: 1, raw_mode: false, raw_expects_backward: false }));
    assert_eq!(publisher.try_publish(BusChannel::Commands, frame), PublishResult::Queued);

    let BusFrame::Confirmation(conf) = conf_obs
        .recv_timeout(Duration::from_millis(500))
        .expect("confirmation")
    else {
        panic!("expected confirmation frame");
    };
    assert_eq!(conf.status, DeliveryStatus::Ok);

    let BusFrame::Event(event) = ev_obs
        .recv_timeout(Duration::from_millis(500))
        .expect("event")
    else {
        panic!("expected event frame");
    };
    assert!(matches!(event.payload, BusEventPayload::DaliEventPayload(_)));
}
