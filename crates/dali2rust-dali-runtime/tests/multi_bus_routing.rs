use std::sync::{Arc, Mutex};
use std::time::Duration;

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost, BusId, BusPublisher, PublishResult};
use dali2rust_dali_runtime::{
    spawn_dali_worker, DaliController, DaliRuntimeConfig, DaliWorkerCounters, StdClock,
};
use dali2rust_domain::registry::RegistryReadPort;
use dali2rust_registry_runtime::RegistryStore;
use dali2rust_test_support::wait_until;

struct RoutedWorkers {
    publisher: BusPublisher,
    transport_1: Arc<Mutex<MockDaliTransport>>,
    transport_2: Arc<Mutex<MockDaliTransport>>,
    _worker_1: std::thread::JoinHandle<()>,
    _worker_2: std::thread::JoinHandle<()>,
    _host: BusHost,
}

fn spawn_routed_workers() -> RoutedWorkers {
    let (host, publisher, (worker_1_cmd, worker_2_cmd)) = BusHost::spawn(
        BusConfig::default(),
        |reg| {
            (
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
            )
        },
    );
    let transport_1 = Arc::new(Mutex::new(MockDaliTransport::new()));
    let transport_2 = Arc::new(Mutex::new(MockDaliTransport::new()));
    let read_port: Arc<dyn RegistryReadPort> = Arc::new(RegistryStore::with_adapter_count(2));
    let worker_1 = spawn_dali_worker(
        worker_1_cmd,
        DaliController::new(Arc::clone(&transport_1), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        Arc::clone(&read_port),
        publisher.clone(),
        BusId(1),
        Arc::new(DaliWorkerCounters::default()),
        Arc::new(dali2rust_platform::dali::WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    );
    let worker_2 = spawn_dali_worker(
        worker_2_cmd,
        DaliController::new(Arc::clone(&transport_2), Box::new(StdClock::new())),
        DaliRuntimeConfig::default(),
        read_port,
        publisher.clone(),
        BusId(2),
        Arc::new(DaliWorkerCounters::default()),
        Arc::new(dali2rust_platform::dali::WireActivity::new()),
        Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
    );
    RoutedWorkers {
        publisher,
        transport_1,
        transport_2,
        _worker_1: worker_1,
        _worker_2: worker_2,
        _host: host,
    }
}

fn publish_targeted_command(publisher: &BusPublisher, correlation_id: u64, target_adapter_id: u16) {
    let frame = BusFrame::command(dali2rust_contracts::bus::command_envelope(0, correlation_id, target_adapter_id, None, dali2rust_contracts::msg::DaliCommandPayload { wire_address: 0, command: 144, repeat_count: 1, raw_mode: false, raw_expects_backward: false }));
    assert_eq!(publisher.try_publish(BusChannel::Commands, frame), PublishResult::Queued);
}

fn wait_for_frames(transport: &Arc<Mutex<MockDaliTransport>>, expected: usize) {
    wait_until(
        || transport.lock().expect("transport lock").sent_frames().len() == expected,
        Duration::from_millis(500),
    );
}

#[test]
fn bus040_command_for_adapter_1_targets_worker_1() {
    let routed = spawn_routed_workers();
    publish_targeted_command(&routed.publisher, 1, 1);
    wait_for_frames(&routed.transport_1, 1);
    assert_eq!(routed.transport_2.lock().expect("transport 2").sent_frames().len(), 0);
}

#[test]
fn bus041_command_for_adapter_2_targets_worker_2_only() {
    let routed = spawn_routed_workers();
    publish_targeted_command(&routed.publisher, 2, 2);
    wait_for_frames(&routed.transport_2, 1);
    assert_eq!(routed.transport_1.lock().expect("transport 1").sent_frames().len(), 0);
}

#[test]
fn bus042_distinct_targets_for_each_publish() {
    let routed = spawn_routed_workers();
    publish_targeted_command(&routed.publisher, 10, 1);
    publish_targeted_command(&routed.publisher, 11, 2);
    wait_for_frames(&routed.transport_1, 1);
    wait_for_frames(&routed.transport_2, 1);
}
