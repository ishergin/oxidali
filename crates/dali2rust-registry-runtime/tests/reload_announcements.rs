mod support;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_bus::{BusFrame, BusId};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    fixed_text_32, BusEventPayload, OperationWorkerSignal, Origin, RegistrySliceReloadCommand,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_platform::slice_store::{SliceKey, SliceStore};
use dali2rust_registry_runtime::{
    encode_persistence_blob, PersistableGroupRecord, PersistableGroupsSlice,
    PersistableVirtualLampsSlice, PersistableVlRecord, PersistenceEnvelope, StagedSlice,
    GROUPS_SLICE_VERSION, VIRTUAL_LAMPS_SLICE_VERSION,
};
use dali2rust_test_support::temp_slice_store;
use support::{publish_cmd, spawn_registry_stack_with, RegistryStackOptions, RegistryTestStack};

const ADAPTERS: u8 = 2;
const LAMPS_PER_ADAPTER: u8 = 64;
const GROUPS_PER_ADAPTER: u8 = 16;
const OBSERVER_INBOX: usize = 1024;
const IMPORT_WORKFLOW: u64 = 77;
const ANNOUNCED_WITHIN: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);

fn lamps(adapter_id: u8) -> StagedSlice {
    let lamps = (0..LAMPS_PER_ADAPTER)
        .map(|virtual_lamp_id| PersistableVlRecord {
            virtual_lamp_id,
            name: format!("lamp {virtual_lamp_id}"),
            ha_entity_enabled: true,
            binding_short: None,
        })
        .collect();
    let slice = PersistableVirtualLampsSlice { adapter_id, lamps };
    let envelope = PersistenceEnvelope::new(VIRTUAL_LAMPS_SLICE_VERSION, slice);
    let bytes = encode_persistence_blob(&envelope).expect("encode lamps");
    StagedSlice { key: SliceKey::VirtualLamps { adapter_id }, bytes }
}

fn groups(adapter_id: u8) -> StagedSlice {
    let groups = (0..GROUPS_PER_ADAPTER)
        .map(|group_id| PersistableGroupRecord {
            group_id,
            name: format!("group {group_id}"),
            ha_entity_enabled: true,
        })
        .collect();
    let slice = PersistableGroupsSlice { adapter_id, groups, rows: Vec::new() };
    let envelope = PersistenceEnvelope::new(GROUPS_SLICE_VERSION, slice);
    let bytes = encode_persistence_blob(&envelope).expect("encode groups");
    StagedSlice { key: SliceKey::Groups { adapter_id }, bytes }
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum Heard {
    Lamp(u8, u8),
    Group(u8, u8),
    GroupMatrix(u8),
}

fn heard(payload: &BusEventPayload) -> Option<Heard> {
    Some(match payload {
        BusEventPayload::VirtualLampChangedEvent(ev) => Heard::Lamp(ev.adapter_id, ev.virtual_lamp_id),
        BusEventPayload::GroupChangedEvent(ev) => Heard::Group(ev.adapter_id, ev.group_id),
        BusEventPayload::GroupMatrixChangedEvent(ev) => Heard::GroupMatrix(ev.adapter_id),
        _ => return None,
    })
}

fn expected() -> HashSet<Heard> {
    let per_adapter = |adapter_id| {
        let lamps = (0..LAMPS_PER_ADAPTER).map(move |id| Heard::Lamp(adapter_id, id));
        let groups = (0..GROUPS_PER_ADAPTER).map(move |id| Heard::Group(adapter_id, id));
        lamps.chain(groups).chain(std::iter::once(Heard::GroupMatrix(adapter_id)))
    };
    (0..ADAPTERS).flat_map(per_adapter).collect()
}

fn commit(stack: &RegistryTestStack) {
    let slices = (0..ADAPTERS).flat_map(|adapter_id| [lamps(adapter_id), groups(adapter_id)]);
    stack.store.stage_import(IMPORT_WORKFLOW, slices.collect()).expect("stage");
    let reload = RegistrySliceReloadCommand { slice_name: fixed_text_32("virtual_lamps") };
    let target = BusId::default().0;
    let envelope = command_envelope(SOURCE_ID_UNSPECIFIED, IMPORT_WORKFLOW, target, Some(Origin::Api), reload);
    publish_cmd(&stack.publisher, envelope);
}

fn listen(stack: &RegistryTestStack) -> (HashSet<Heard>, bool) {
    let (mut announced, mut succeeded) = (HashSet::new(), false);
    let deadline = Instant::now() + ANNOUNCED_WITHIN;
    while Instant::now() < deadline && !(succeeded && announced == expected()) {
        let Ok(BusFrame::Event(event)) = stack.ev_rx.recv_timeout(POLL) else {
            continue;
        };
        if let BusEventPayload::OperationWorkerSignalEvent(signal) = &event.payload {
            succeeded |= signal.signal == OperationWorkerSignal::WorkerSucceeded;
        }
        announced.extend(heard(&event.payload));
    }
    (announced, succeeded)
}

#[test]
fn an_import_of_more_records_than_the_events_ingress_holds_announces_each_without_an_overflow() {
    let slices: Arc<dyn SliceStore> = Arc::new(temp_slice_store("reload-announcements"));
    let stack = spawn_registry_stack_with(RegistryStackOptions {
        adapter_count: ADAPTERS,
        event_observer_capacity: OBSERVER_INBOX,
        slices: Some(slices),
        ..RegistryStackOptions::default()
    });
    commit(&stack);
    let (announced, succeeded) = listen(&stack);
    assert!(succeeded, "the import did not succeed");
    let missing: Vec<_> = expected().into_iter().filter(|record| !announced.contains(record)).collect();
    assert!(missing.is_empty(), "records never announced: {missing:?}");
    let overflow = stack.publisher.counters_snapshot().events.ingress_overflow;
    assert_eq!(overflow, 0, "the announcements overflowed the events ingress");
}
