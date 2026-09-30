use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusFrame, BusId, PublishResult};
use dali2rust_contracts::msg::{
    fixed_text_32, BusEventPayload, DeliveryStatus, ErrorCode, HclAlgorithm, HclLevelMode,
    HclPointList, HclSchedulePointRow, HclScheduleDeleteCommand, HclScheduleEnableCommand,
    HclScheduleUpsertCommand, HclTargetList, HclTargetRow, HclTargetScope, HclTimeRef,
};
use dali2rust_domain::registry::HclScheduleReadPort;
use dali2rust_registry_runtime::{RegistryStore, RegistryWorkerCounters};
use dali2rust_test_support::wait_until;

mod support;
use support::recv_confirm_for;

const ALL_DAYS: u8 = 0b0111_1111;
const MOSCOW_LAT_MICRODEG: i32 = 55_755_800;
const MOSCOW_LON_MICRODEG: i32 = 37_617_300;

fn spawn_stack() -> (
    dali2rust_bus::BusPublisher,
    std::sync::mpsc::Receiver<BusFrame>,
    std::sync::mpsc::Receiver<BusFrame>,
    Arc<RegistryStore>,
    Arc<RegistryWorkerCounters>,
    dali2rust_bus::BusHost,
) {
    let s = support::spawn_registry_stack(1, 32);
    (s.publisher, s.conf_rx, s.ev_rx, s.store, s.counters, s._host)
}

fn target(adapter_id: u8, group_mask: u16) -> HclTargetRow {
    HclTargetRow {
        adapter_id,
        scope: HclTargetScope::Group,
        group_mask,
    }
}

fn point(offset_minutes: i16, level: u8, kelvin: u16) -> HclSchedulePointRow {
    HclSchedulePointRow {
        time_ref: HclTimeRef::Absolute,
        offset_minutes,
        level_mode: HclLevelMode::Absolute,
        level: Some(level),
        color_temperature_kelvin: Some(kelvin),
    }
}

fn chunk(
    schedule_id: &str,
    first_target_index: u8,
    targets: &[HclTargetRow],
    first_point_index: u8,
    points: &[HclSchedulePointRow],
    last_chunk: bool,
) -> HclScheduleUpsertCommand {
    HclScheduleUpsertCommand {
        schedule_id: fixed_text_32(schedule_id),
        enabled: true,
        algorithm: HclAlgorithm::Interpolated,
        active_days_mask: ALL_DAYS,
        latitude_microdeg: Some(MOSCOW_LAT_MICRODEG),
        longitude_microdeg: Some(MOSCOW_LON_MICRODEG),
        first_target_index,
        targets: HclTargetList::from_slice(targets).expect("targets fit the chunk"),
        first_point_index,
        points: HclPointList::from_slice(points).expect("points fit the chunk"),
        last_chunk,
    }
}

fn publish<P>(publisher: &dali2rust_bus::BusPublisher, correlation_id: u64, payload: P)
where
    P: Into<dali2rust_contracts::msg::BusCommandPayload>,
{
    let envelope = dali2rust_contracts::bus::command_envelope(
        0,
        correlation_id,
        BusId::default().0,
        None,
        payload,
    );
    assert_eq!(
        publisher.try_publish(BusChannel::Commands, BusFrame::command(envelope)),
        PublishResult::Queued
    );
}

fn assert_ok(conf: &dali2rust_contracts::msg::ConfirmationEnvelope) {
    assert_eq!(conf.status, DeliveryStatus::Ok);
    assert!(conf.confirmation.error.is_none());
}

fn assert_failed(conf: &dali2rust_contracts::msg::ConfirmationEnvelope, message: &str) {
    assert_eq!(conf.status, DeliveryStatus::ExecutionFailed);
    let error = conf.confirmation.error.as_ref().expect("product error");
    assert_eq!(error.message.as_str(), message);
}

fn recv_schedule_changed(
    rx: &std::sync::mpsc::Receiver<BusFrame>,
    correlation_id: u64,
) -> dali2rust_contracts::msg::HclScheduleChangedEvent {
    for _ in 0..40 {
        let Ok(BusFrame::Event(ev)) = rx.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        if ev.meta.correlation_id != correlation_id {
            continue;
        }
        if let BusEventPayload::HclScheduleChangedEvent(body) = &ev.payload {
            return body.clone();
        }
    }
    panic!("no HclScheduleChangedEvent for correlation {correlation_id}");
}

#[test]
fn schedule_is_invisible_until_its_last_chunk_commits() {
    let (publisher, conf_rx, ev_rx, store, counters, _host) = spawn_stack();

    publish(
        &publisher,
        1,
        chunk("morning", 0, &[target(0, 0b10)], 0, &[point(360, 80, 2700)], false),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 1));
    assert!(
        store.hcl_schedule_view("morning").is_none(),
        "a staged schedule must not be readable"
    );

    publish(
        &publisher,
        2,
        chunk("morning", 1, &[target(1, 0b1000)], 1, &[point(480, 200, 4000)], true),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 2));

    let changed = recv_schedule_changed(&ev_rx, 2);
    assert_eq!(changed.schedule_id.as_str(), "morning");
    assert!(!changed.removed);
    assert!(changed.enabled);

    let view = store.hcl_schedule_view("morning").expect("committed schedule");
    assert_eq!(view.algorithm, HclAlgorithm::Interpolated);
    assert_eq!(view.targets, vec![target(0, 0b10), target(1, 0b1000)]);
    assert_eq!(
        view.points,
        vec![point(360, 80, 2700), point(480, 200, 4000)]
    );
    assert_eq!(
        counters
            .command
            .hcl_schedule_upserts_applied
            .load(Ordering::Relaxed),
        1,
        "one commit, not one per chunk"
    );
}

#[test]
fn a_staged_id_is_taken_even_though_it_is_not_readable() {
    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_stack();

    publish(
        &publisher,
        1,
        chunk("evening", 0, &[target(0, 0b10)], 0, &[point(360, 80, 2700)], false),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 1));

    assert!(
        store.hcl_schedule_view("evening").is_none(),
        "a staged schedule must not be readable"
    );
    assert!(
        store.hcl_schedule_id_taken("evening"),
        "a staged id is spoken for: a second create must not pick it"
    );
    assert!(
        !store.hcl_schedule_id_taken("never-sent"),
        "an id nobody staged or committed is free"
    );

    publish(
        &publisher,
        2,
        chunk("evening", 1, &[target(1, 0b1000)], 1, &[point(480, 200, 4000)], true),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 2));

    assert!(
        store.hcl_schedule_id_taken("evening"),
        "and it stays taken once committed"
    );
}

#[test]
fn replacing_a_schedule_keeps_the_old_one_readable_until_the_commit() {
    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_stack();

    publish(
        &publisher,
        11,
        chunk("evening", 0, &[target(0, 0b1)], 0, &[point(1200, 40, 2200)], true),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 11));

    publish(
        &publisher,
        12,
        chunk("evening", 0, &[target(0, 0b100)], 0, &[point(600, 254, 5000)], false),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 12));

    let view = store.hcl_schedule_view("evening").expect("previous schedule");
    assert_eq!(
        view.points,
        vec![point(1200, 40, 2200)],
        "the stored curve stays intact while its replacement stages"
    );

    publish(
        &publisher,
        13,
        chunk("evening", 1, &[], 1, &[point(660, 100, 3000)], true),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 13));

    let view = store.hcl_schedule_view("evening").expect("replacement");
    assert_eq!(
        view.points,
        vec![point(600, 254, 5000), point(660, 100, 3000)],
        "the commit replaces the whole schedule, it does not merge into it"
    );
}

#[test]
fn a_gap_in_the_chunk_sequence_drops_it_instead_of_half_applying() {
    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_stack();

    publish(
        &publisher,
        21,
        chunk("noon", 0, &[target(0, 0b1)], 0, &[point(0, 10, 2700)], false),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 21));

    publish(
        &publisher,
        22,
        chunk("noon", 2, &[target(0, 0b10)], 2, &[point(120, 20, 2700)], false),
    );
    assert_failed(&recv_confirm_for(&conf_rx, 22), "chunk_out_of_order");

    publish(
        &publisher,
        23,
        chunk("noon", 1, &[], 1, &[point(240, 30, 2700)], true),
    );
    assert_failed(&recv_confirm_for(&conf_rx, 23), "chunk_out_of_order");
    assert!(store.hcl_schedule_view("noon").is_none());
}

#[test]
fn delete_removes_the_schedule_and_publishes_the_removal() {
    let (publisher, conf_rx, ev_rx, store, counters, _host) = spawn_stack();

    publish(
        &publisher,
        31,
        chunk("dusk", 0, &[target(0, 0b1)], 0, &[point(1080, 60, 2400)], true),
    );
    assert_ok(&recv_confirm_for(&conf_rx, 31));
    let revision_before = store.hcl_schedules_revision();

    publish(
        &publisher,
        32,
        HclScheduleDeleteCommand {
            schedule_id: fixed_text_32("dusk"),
        },
    );
    assert_ok(&recv_confirm_for(&conf_rx, 32));

    let changed = recv_schedule_changed(&ev_rx, 32);
    assert_eq!(changed.schedule_id.as_str(), "dusk");
    assert!(changed.removed);
    assert!(store.hcl_schedule_view("dusk").is_none());
    assert!(store.list_hcl_schedule_views().is_empty());
    assert_ne!(store.hcl_schedules_revision(), revision_before);
    assert_eq!(
        counters
            .command
            .hcl_schedule_deletes_applied
            .load(Ordering::Relaxed),
        1
    );

    publish(
        &publisher,
        33,
        HclScheduleDeleteCommand {
            schedule_id: fixed_text_32("never-existed"),
        },
    );
    let conf = recv_confirm_for(&conf_rx, 33);
    assert_failed(&conf, "schedule_not_found");
    assert_eq!(
        conf.confirmation.error.as_ref().map(|e| e.code),
        Some(ErrorCode::NotFound)
    );
}

fn switch(schedule_id: &str, enabled: bool) -> HclScheduleEnableCommand {
    HclScheduleEnableCommand {
        schedule_id: fixed_text_32(schedule_id),
        enabled,
    }
}

fn store_porch(publisher: &dali2rust_bus::BusPublisher, conf_rx: &std::sync::mpsc::Receiver<BusFrame>, corr: u64) {
    publish(
        publisher,
        corr,
        chunk("porch", 0, &[target(0, 0b1)], 0, &[point(1080, 60, 2400)], true),
    );
    assert_ok(&recv_confirm_for(conf_rx, corr));
}

#[test]
fn a_switch_moves_only_the_enabled_bit_and_publishes_it() {
    let (publisher, conf_rx, ev_rx, store, _counters, _host) = spawn_stack();
    store_porch(&publisher, &conf_rx, 71);
    let revision_before = store.hcl_schedules_revision();

    publish(&publisher, 72, switch("porch", false));
    assert_ok(&recv_confirm_for(&conf_rx, 72));
    let changed = recv_schedule_changed(&ev_rx, 72);
    assert_eq!(changed.schedule_id.as_str(), "porch");
    assert!(!changed.removed);
    assert!(!changed.enabled, "the event carries the bit the registry now holds");
    assert!(!store.hcl_schedule_view("porch").expect("still stored").enabled);
    assert_ne!(store.hcl_schedules_revision(), revision_before);

    publish(&publisher, 73, switch("porch", true));
    assert_ok(&recv_confirm_for(&conf_rx, 73));
    assert!(recv_schedule_changed(&ev_rx, 73).enabled);
    let view = store.hcl_schedule_view("porch").expect("still stored");
    assert!(view.enabled);
    assert_eq!(view.targets, vec![target(0, 0b1)], "the switch leaves the targets alone");
    assert_eq!(view.points, vec![point(1080, 60, 2400)], "and the curve");
}

#[test]
fn a_switch_to_the_bit_already_stored_confirms_and_wakes_nobody() {
    let (publisher, conf_rx, ev_rx, store, _counters, _host) = spawn_stack();
    store_porch(&publisher, &conf_rx, 81);
    let revision_before = store.hcl_schedules_revision();

    publish(&publisher, 82, switch("porch", true));
    assert_ok(&recv_confirm_for(&conf_rx, 82));
    assert_eq!(store.hcl_schedules_revision(), revision_before);

    publish(&publisher, 83, switch("porch", false));
    assert_ok(&recv_confirm_for(&conf_rx, 83));
    let mut seen = Vec::new();
    while let Ok(BusFrame::Event(ev)) = ev_rx.recv_timeout(Duration::from_secs(3)) {
        if let BusEventPayload::HclScheduleChangedEvent(body) = &ev.payload {
            seen.push((ev.meta.correlation_id, body.enabled));
            if ev.meta.correlation_id == 83 {
                break;
            }
        }
    }
    assert_eq!(
        seen.iter().filter(|(corr, _)| *corr != 81).copied().collect::<Vec<_>>(),
        vec![(83, false)],
        "the switch that changed nothing published nothing ahead of the one that did"
    );
}

#[test]
fn switching_a_schedule_that_does_not_exist_is_refused_by_name() {
    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_stack();
    store_porch(&publisher, &conf_rx, 91);

    publish(&publisher, 92, switch("never-existed", false));
    let conf = recv_confirm_for(&conf_rx, 92);
    assert_failed(&conf, "schedule_not_found");
    assert_eq!(
        conf.confirmation.error.as_ref().map(|e| e.code),
        Some(ErrorCode::NotFound)
    );
    assert!(
        store.hcl_schedule_view("porch").expect("untouched").enabled,
        "a refused switch moves no other schedule"
    );
}

#[test]
fn a_point_the_scheduler_could_not_resolve_is_refused() {
    let (publisher, conf_rx, _ev_rx, store, _counters, _host) = spawn_stack();

    let mut sunset_without_location = chunk(
        "astro",
        0,
        &[target(0, 0b1)],
        0,
        &[HclSchedulePointRow {
            time_ref: HclTimeRef::Sunset,
            offset_minutes: -30,
            level_mode: HclLevelMode::LastActive,
            level: None,
            color_temperature_kelvin: Some(3000),
        }],
        true,
    );
    sunset_without_location.latitude_microdeg = None;
    sunset_without_location.longitude_microdeg = None;
    publish(&publisher, 41, sunset_without_location);
    assert_failed(&recv_confirm_for(&conf_rx, 41), "point_needs_location");

    let levelless = chunk(
        "levelless",
        0,
        &[target(0, 0b1)],
        0,
        &[HclSchedulePointRow {
            time_ref: HclTimeRef::Absolute,
            offset_minutes: 600,
            level_mode: HclLevelMode::Absolute,
            level: None,
            color_temperature_kelvin: Some(3000),
        }],
        true,
    );
    publish(&publisher, 42, levelless);
    assert_failed(&recv_confirm_for(&conf_rx, 42), "point_level_invalid");

    let out_of_day = chunk("late", 0, &[target(0, 0b1)], 0, &[point(1440, 10, 2700)], true);
    publish(&publisher, 43, out_of_day);
    assert_failed(
        &recv_confirm_for(&conf_rx, 43),
        "point_offset_out_of_range",
    );

    let empty_mask = chunk("nobody", 0, &[target(0, 0)], 0, &[point(600, 10, 2700)], true);
    publish(&publisher, 44, empty_mask);
    assert_failed(&recv_confirm_for(&conf_rx, 44), "target_group_mask_empty");

    assert!(store.list_hcl_schedule_views().is_empty());
}

mod persistence {
    use super::*;
    use dali2rust_bsp::slice_store_files::InMemorySliceStore;
    use dali2rust_platform::slice_store::{SliceKey, SliceStore};

    fn spawn_stack_with_slices(
        slices: Arc<dyn SliceStore>,
    ) -> (
        dali2rust_bus::BusPublisher,
        std::sync::mpsc::Receiver<BusFrame>,
        Arc<RegistryStore>,
        dali2rust_bus::BusHost,
    ) {
        let s = support::spawn_registry_stack_with(support::RegistryStackOptions {
            slices: Some(slices),
            ..support::RegistryStackOptions::default()
        });
        (s.publisher, s.conf_rx, s.store, s._host)
    }

    #[test]
    fn a_committed_schedule_survives_a_restart() {
        let slices: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
        let (publisher, conf_rx, store, _host) = spawn_stack_with_slices(Arc::clone(&slices));

        publish(
            &publisher,
            51,
            chunk(
                "morning",
                0,
                &[target(0, 0b1010)],
                0,
                &[point(360, 80, 2700), point(480, 200, 4000)],
                true,
            ),
        );
        assert_ok(&recv_confirm_for(&conf_rx, 51));
        wait_until(
            || slices.load(SliceKey::HclSchedules).is_ok(),
            Duration::from_secs(3),
        );

        let restarted = RegistryStore::with_adapter_count(1);
        let report = restarted.hydrate_from_store(slices.as_ref(), 1);
        assert!(report.is_ok(), "hydrate errors: {:?}", report.errors);

        let view = restarted
            .hcl_schedule_view("morning")
            .expect("the schedule should come back");
        assert_eq!(view.algorithm, HclAlgorithm::Interpolated);
        assert_eq!(view.active_days_mask, ALL_DAYS);
        assert_eq!(view.targets, vec![target(0, 0b1010)]);
        assert_eq!(view.points, vec![point(360, 80, 2700), point(480, 200, 4000)]);
        assert_eq!(view.latitude_microdeg, Some(MOSCOW_LAT_MICRODEG));
        drop(store);
    }

    #[test]
    fn a_deleted_schedule_stays_deleted_across_a_restart() {
        let slices: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
        let (publisher, conf_rx, store, _host) = spawn_stack_with_slices(Arc::clone(&slices));

        publish(
            &publisher,
            61,
            chunk("dusk", 0, &[target(0, 0b1)], 0, &[point(1080, 60, 2400)], true),
        );
        assert_ok(&recv_confirm_for(&conf_rx, 61));
        wait_until(
            || slices.load(SliceKey::HclSchedules).is_ok(),
            Duration::from_secs(3),
        );

        publish(
            &publisher,
            62,
            HclScheduleDeleteCommand {
                schedule_id: fixed_text_32("dusk"),
            },
        );
        assert_ok(&recv_confirm_for(&conf_rx, 62));
        wait_until(
            || {
                slices
                    .load(SliceKey::HclSchedules)
                    .map(|bytes| !String::from_utf8_lossy(&bytes).contains("dusk"))
                    .unwrap_or(false)
            },
            Duration::from_secs(3),
        );

        let restarted = RegistryStore::with_adapter_count(1);
        restarted.hydrate_from_store(slices.as_ref(), 1);
        assert!(
            restarted.list_hcl_schedule_views().is_empty(),
            "a deleted schedule must not come back from flash"
        );
        drop(store);
    }

    #[test]
    fn switches_apply_at_once_while_flash_keeps_the_last_written_bit() {
        let slices: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
        let s = support::spawn_registry_stack_with(support::RegistryStackOptions {
            slices: Some(Arc::clone(&slices)),
            ..support::RegistryStackOptions::default()
        });
        store_porch(&s.publisher, &s.conf_rx, 101);
        wait_until(
            || slices.load(SliceKey::HclSchedules).is_ok(),
            Duration::from_secs(3),
        );

        for (corr, enabled) in [(102, false), (103, true), (104, false)] {
            publish(&s.publisher, corr, switch("porch", enabled));
            assert_ok(&recv_confirm_for(&s.conf_rx, corr));
        }
        let settled = 105;
        publish(&s.publisher, settled, switch("porch", false));
        assert_ok(&recv_confirm_for(&s.conf_rx, settled));
        let restarted_bit = || {
            let restarted = RegistryStore::with_adapter_count(1);
            restarted.hydrate_from_store(slices.as_ref(), 1);
            restarted.hcl_schedule_view("porch").map(|view| view.enabled)
        };
        assert!(!s.store.hcl_schedule_view("porch").expect("stored").enabled);
        assert_eq!(restarted_bit(), Some(true), "a restart now keeps the bit last written");
        assert_eq!(
            s.counters.command.hcl_switches_coalesced.load(Ordering::Relaxed),
            2,
            "the two switches after the first joined its waiting write"
        );

        drop(s._host);
        wait_until(|| restarted_bit() == Some(false), Duration::from_secs(3));
        assert_eq!(restarted_bit(), Some(false), "a stopping worker writes what waits");
    }

    #[test]
    fn the_worst_case_schedule_set_fits_its_slot() {
        let slices: Arc<dyn SliceStore> = Arc::new(InMemorySliceStore::new());
        let (publisher, conf_rx, store, _host) = spawn_stack_with_slices(Arc::clone(&slices));

        let mut correlation = 100u64;
        for index in 0..8u8 {
            let schedule_id = format!("schedule-{index}");
            write_worst_case_schedule(&publisher, &conf_rx, &mut correlation, &schedule_id);
        }
        wait_until(
            || {
                slices
                    .load(SliceKey::HclSchedules)
                    .map(|bytes| String::from_utf8_lossy(&bytes).contains("schedule-7"))
                    .unwrap_or(false)
            },
            Duration::from_secs(5),
        );

        let bytes = slices.load(SliceKey::HclSchedules).expect("stored slice");
        let capacity = dali2rust_bsp::slice_layout::SMALL_SLOT_PAYLOAD_BYTES as usize;
        assert!(
            bytes.len() < capacity,
            "the worst-case schedule set is {} B, past the {capacity} B slot payload",
            bytes.len()
        );
        let restarted = RegistryStore::with_adapter_count(1);
        restarted.hydrate_from_store(slices.as_ref(), 1);
        assert_eq!(restarted.list_hcl_schedule_views().len(), 8);
        drop(store);
    }

    fn write_worst_case_schedule(
        publisher: &dali2rust_bus::BusPublisher,
        conf_rx: &std::sync::mpsc::Receiver<BusFrame>,
        correlation: &mut u64,
        schedule_id: &str,
    ) {
        let targets: Vec<HclTargetRow> = (0..16).map(|i| target(i % 8, 1 << (i % 16))).collect();
        let points: Vec<HclSchedulePointRow> = (0..24)
            .map(|i| point(i as i16 * 60, (i as u8) % 255, 2000 + i as u16 * 100))
            .collect();
        let chunk_count = 12;
        for index in 0..chunk_count {
            let target_start = (index * 4).min(targets.len());
            let target_end = (target_start + 4).min(targets.len());
            let point_start = (index * 2).min(points.len());
            let point_end = (point_start + 2).min(points.len());
            *correlation += 1;
            publish(
                publisher,
                *correlation,
                chunk(
                    schedule_id,
                    target_start as u8,
                    &targets[target_start..target_end],
                    point_start as u8,
                    &points[point_start..point_end],
                    index + 1 == chunk_count,
                ),
            );
            assert_ok(&recv_confirm_for(conf_rx, *correlation));
        }
    }
}
