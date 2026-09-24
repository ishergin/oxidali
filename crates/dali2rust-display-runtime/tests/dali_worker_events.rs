use std::sync::Arc;
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, PublishResult};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
};
use dali2rust_display_runtime::{
    spawn_display_worker, DisplaySample, DisplayView, HardwareDisplay, StaticDisplaySource,
    DISPLAY_WORKER_HANDLED_EVENTS,
};
use dali2rust_registry_runtime::RegistryStore;
use dali2rust_test_support::wait_until;

#[test]
fn display_view_reflects_dali_worker_traffic() {
    let (_host, publisher, (worker_cmd, disp_ev)) = BusHost::spawn(BusConfig::default(), |reg| {
        (
            reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
            reg.subscribe_events(8, DISPLAY_WORKER_HANDLED_EVENTS),
        )
    });
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
    let view = Arc::new(DisplayView::default());
    let _display = spawn_display_worker(
        disp_ev,
        Arc::new(StaticDisplaySource(DisplaySample {
            gear_known: 1,
            adapter_enabled: true,
            time_synced: true,
            ..DisplaySample::default()
        })),
        Arc::clone(&view),
        HardwareDisplay::none(),
    );
    let setpoint = dali2rust_contracts::msg::LightSetpoint {
        power: dali2rust_contracts::msg::PowerState::On,
        level: 180,
        color: None,
    };
    let frame = BusFrame::command(dali2rust_contracts::bus::command_envelope(
        0,
        1,
        BusId::default().0,
        None,
        dali2rust_contracts::msg::DaliSetTargetStateCommand::for_short(0, 3, &setpoint),
    ));
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, frame),
        PublishResult::Queued
    );
    wait_until(
        || view.lines()[6].text.contains("A03 ON 180"),
        Duration::from_secs(5),
    );
}
