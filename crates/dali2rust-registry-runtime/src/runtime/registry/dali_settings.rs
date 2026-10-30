use dali2rust_contracts::msg::{
    BusUnitNvmObservedEvent, BusUnitNvmRow, BusUnitQuiescenceObservedEvent, DaliSettingsUpdateCommand,
};
use std::time::{Duration, Instant};

use dali2rust_domain::dali::dev103::bus_unit::{is_newer, FACTORY_RANDOM_ADDRESS, STANDARD_OPERATING_MODE};
use dali2rust_domain::registry::{ApplicationActiveMover, DaliSettingsReadPort, DaliSettingsView};
use dali2rust_platform::arbitration::AnswerInputs;
use dali2rust_platform::bus_unit::{BusUnitNvm, BusUnitSnapshot};

use super::persistence_slices::{
    decode_versioned_slice, DaliSettingsBeforeBusUnitSlice, PersistableDaliSettingsSlice,
    SliceDecodeError, DALI_SETTINGS_SLICE_VERSION,
};
use super::store::{Inner, RegistryStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DaliSettingsRecord {
    pub dt8_auto_activation_repair: bool,
    pub dt8_rgbwaf_control_assert: bool,
    pub application_active: bool,
    pub device_short_address: Option<u8>,
    pub random_address: u32,
    pub device_groups: u32,
    pub operating_mode: u8,
    pub power_cycle_notification: bool,
    pub application_active_moved_by: ApplicationActiveMover,
    pub bus_unit_revision: u32,
    pub wire_acknowledged: u32,
    wire_applied: Option<(u32, BusUnitNvmRow)>,
    operator_moved_at: [Option<u32>; UNIT_FIELDS],
    quiescent_until: Option<Instant>,
    quiescence_acknowledged: u32,
}

#[derive(Clone, Copy)]
enum UnitField {
    ShortAddress,
    RandomAddress,
    DeviceGroups,
    OperatingMode,
    PowerCycleNotification,
    ApplicationActive,
}

const UNIT_FIELDS: usize = UnitField::ApplicationActive as usize + 1;

fn differs(a: &BusUnitNvmRow, b: &BusUnitNvmRow, field: UnitField) -> bool {
    match field {
        UnitField::ShortAddress => a.short_address != b.short_address,
        UnitField::RandomAddress => a.random_address != b.random_address,
        UnitField::DeviceGroups => a.device_groups != b.device_groups,
        UnitField::OperatingMode => a.operating_mode != b.operating_mode,
        UnitField::PowerCycleNotification => a.power_cycle_notification != b.power_cycle_notification,
        UnitField::ApplicationActive => a.application_active != b.application_active,
    }
}

const EVERY_UNIT_FIELD: [UnitField; UNIT_FIELDS] = [
    UnitField::ShortAddress,
    UnitField::RandomAddress,
    UnitField::DeviceGroups,
    UnitField::OperatingMode,
    UnitField::PowerCycleNotification,
    UnitField::ApplicationActive,
];

impl Default for DaliSettingsRecord {
    // IEC 62386-103 Table 17
    fn default() -> Self {
        Self {
            dt8_auto_activation_repair: true,
            dt8_rgbwaf_control_assert: true,
            application_active: true,
            device_short_address: None,
            random_address: FACTORY_RANDOM_ADDRESS,
            device_groups: 0,
            operating_mode: STANDARD_OPERATING_MODE,
            power_cycle_notification: false,
            application_active_moved_by: ApplicationActiveMover::Unmoved,
            bus_unit_revision: 0,
            wire_acknowledged: 0,
            wire_applied: None,
            operator_moved_at: [None; UNIT_FIELDS],
            quiescent_until: None,
            quiescence_acknowledged: 0,
        }
    }
}

impl DaliSettingsRecord {
    fn to_view(self) -> DaliSettingsView {
        DaliSettingsView {
            dt8_auto_activation_repair: self.dt8_auto_activation_repair,
            dt8_rgbwaf_control_assert: self.dt8_rgbwaf_control_assert,
            application_active: self.application_active,
            device_short_address: self.device_short_address,
        }
    }

    #[must_use]
    pub const fn bus_unit_nvm(&self) -> BusUnitNvmRow {
        BusUnitNvmRow {
            short_address: self.device_short_address,
            random_address: self.random_address,
            device_groups: self.device_groups,
            operating_mode: self.operating_mode,
            power_cycle_notification: self.power_cycle_notification,
            application_active: self.application_active,
        }
    }

    const fn bus_unit_snapshot(&self) -> BusUnitSnapshot {
        BusUnitSnapshot {
            nvm: BusUnitNvm {
                short_address: self.device_short_address,
                random_address: self.random_address,
                device_groups: self.device_groups,
                operating_mode: self.operating_mode,
                power_cycle_notification: self.power_cycle_notification,
                application_active: self.application_active,
            },
            revision: self.bus_unit_revision,
            acknowledged_wire_sequence: self.wire_acknowledged,
            acknowledged_quiescence: self.quiescence_acknowledged,
        }
    }

    fn quiescent_remaining_ms(&self, now: Instant) -> Option<u32> {
        let left = self.quiescent_until?.checked_duration_since(now)?;
        Some(u32::try_from(left.as_millis()).unwrap_or(u32::MAX)).filter(|&ms| ms > 0)
    }

    fn apply_patch(&mut self, body: &DaliSettingsUpdateCommand) {
        let was_active = self.application_active;
        let was_unit = self.bus_unit_nvm();
        if body.patch_mask & DaliSettingsUpdateCommand::PATCH_DT8_AUTO_ACTIVATION_REPAIR != 0 {
            self.dt8_auto_activation_repair = body.dt8_auto_activation_repair;
        }
        if body.patch_mask & DaliSettingsUpdateCommand::PATCH_DT8_RGBWAF_CONTROL_ASSERT != 0 {
            self.dt8_rgbwaf_control_assert = body.dt8_rgbwaf_control_assert;
        }
        if body.patch_mask & DaliSettingsUpdateCommand::PATCH_APPLICATION_ACTIVE != 0 {
            self.application_active = body.application_active;
        }
        if body.patch_mask & DaliSettingsUpdateCommand::PATCH_DEVICE_SHORT_ADDRESS != 0 {
            self.device_short_address =
                (body.device_short_address <= 63).then_some(body.device_short_address);
        }
        if self.application_active != was_active {
            self.application_active_moved_by = ApplicationActiveMover::Command;
        }
        let now_unit = self.bus_unit_nvm();
        if now_unit != was_unit {
            self.bus_unit_revision = self.bus_unit_revision.wrapping_add(1);
        }
        for field in EVERY_UNIT_FIELD {
            if differs(&now_unit, &was_unit, field) {
                self.operator_moved_at[field as usize] = Some(self.bus_unit_revision);
            }
        }
    }

    // IEC 62386-103 §9.2.3
    fn wire_may_move(&self, base: &BusUnitNvmRow, body: &BusUnitNvmObservedEvent, field: UnitField) -> bool {
        let held = self.operator_moved_at[field as usize]
            .is_some_and(|at| is_newer(at, body.adopted_revision));
        differs(&body.observed, base, field) && !held
    }

    fn set_from_wire(&mut self, observed: &BusUnitNvmRow, field: UnitField) {
        match field {
            UnitField::ShortAddress => self.device_short_address = observed.short_address,
            UnitField::RandomAddress => self.random_address = observed.random_address,
            UnitField::DeviceGroups => self.device_groups = observed.device_groups,
            UnitField::OperatingMode => self.operating_mode = observed.operating_mode,
            UnitField::PowerCycleNotification => {
                self.power_cycle_notification = observed.power_cycle_notification;
            }
            UnitField::ApplicationActive => {
                self.application_active = observed.application_active;
                self.application_active_moved_by = ApplicationActiveMover::Wire;
            }
        }
    }

    fn apply_wire(&mut self, body: &BusUnitNvmObservedEvent) -> bool {
        let base = self
            .wire_applied
            .filter(|(revision, _)| *revision == body.adopted_revision)
            .map_or(body.adopted, |(_, observed)| observed);
        let before = self.bus_unit_nvm();
        for field in EVERY_UNIT_FIELD {
            if self.wire_may_move(&base, body, field) {
                self.set_from_wire(&body.observed, field);
            }
        }
        self.wire_applied = Some((body.adopted_revision, body.observed));
        self.wire_acknowledged = body.wire_sequence;
        self.bus_unit_revision = self.bus_unit_revision.wrapping_add(1);
        self.bus_unit_nvm() != before
    }
}

impl RegistryStore {
    pub(crate) fn apply_dali_settings_from_command(
        &self,
        body: &DaliSettingsUpdateCommand,
    ) -> DaliSettingsRecord {
        let mut g = self.write_inner();
        g.dali_settings.apply_patch(body);
        let applied = g.dali_settings;
        drop(g);
        self.dirty.mark_dali_settings_dirty();
        self.publish_bus_unit();
        applied
    }

    pub(crate) fn dali_settings_row(&self) -> DaliSettingsRecord {
        self.read_inner().dali_settings
    }

    pub fn install_bus_unit_sink(&self, sink: std::sync::Arc<AnswerInputs>) {
        if self.bus_unit_sink.set(sink).is_ok() {
            self.publish_bus_unit();
        }
    }

    pub(crate) fn publish_bus_unit(&self) {
        if let Some(sink) = self.bus_unit_sink.get() {
            sink.publish_bus_unit(self.dali_settings_row().bus_unit_snapshot());
        }
    }
}

impl RegistryStore {
    // IEC 62386-103 §9.9.3
    pub(crate) fn apply_bus_unit_quiescence(&self, body: &BusUnitQuiescenceObservedEvent, now: Instant) -> bool {
        let mut g = self.write_inner();
        let row = &mut g.dali_settings;
        if is_newer(row.quiescence_acknowledged, body.sequence) {
            return false;
        }
        let was = row.quiescent_remaining_ms(now).is_some();
        row.quiescent_until = body.remaining_ms.map(|remaining| now + Duration::from_millis(u64::from(remaining)));
        row.quiescence_acknowledged = body.sequence;
        let moved = row.quiescent_remaining_ms(now).is_some() != was;
        drop(g);
        self.publish_bus_unit();
        moved
    }

    pub(crate) fn quiescent_remaining_at(&self, now: Instant) -> Option<u32> {
        self.dali_settings_row().quiescent_remaining_ms(now)
    }
}

impl DaliSettingsReadPort for RegistryStore {
    fn dali_settings_view(&self) -> DaliSettingsView {
        self.dali_settings_row().to_view()
    }

    fn quiescent_remaining_ms(&self) -> Option<u32> {
        self.quiescent_remaining_at(Instant::now())
    }

    fn application_active_moved_by(&self) -> (bool, ApplicationActiveMover) {
        let row = self.dali_settings_row();
        (row.application_active, row.application_active_moved_by)
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistableDaliSettingsSlice {
    PersistableDaliSettingsSlice {
        dt8_auto_activation_repair: inner.dali_settings.dt8_auto_activation_repair,
        dt8_rgbwaf_control_assert: inner.dali_settings.dt8_rgbwaf_control_assert,
        application_active: inner.dali_settings.application_active,
        device_short_address: inner.dali_settings.device_short_address,
        random_address: inner.dali_settings.random_address,
        device_groups: inner.dali_settings.device_groups,
        operating_mode: inner.dali_settings.operating_mode,
        power_cycle_notification: inner.dali_settings.power_cycle_notification,
    }
}

pub(crate) fn decode_dali_settings_slice(
    bytes: &[u8],
) -> Result<PersistableDaliSettingsSlice, SliceDecodeError> {
    decode_versioned_slice(bytes, DALI_SETTINGS_SLICE_VERSION).or_else(|whole| {
        decode_versioned_slice(bytes, DALI_SETTINGS_SLICE_VERSION)
            .map(with_factory_bus_unit)
            .map_err(|_| whole)
    })
}

fn with_factory_bus_unit(before: DaliSettingsBeforeBusUnitSlice) -> PersistableDaliSettingsSlice {
    let factory = DaliSettingsRecord::default();
    PersistableDaliSettingsSlice {
        dt8_auto_activation_repair: before.dt8_auto_activation_repair,
        dt8_rgbwaf_control_assert: before.dt8_rgbwaf_control_assert,
        application_active: before.application_active,
        device_short_address: before.device_short_address,
        random_address: factory.random_address,
        device_groups: factory.device_groups,
        operating_mode: factory.operating_mode,
        power_cycle_notification: factory.power_cycle_notification,
    }
}

pub(crate) fn hydrate_dali_settings_inner(
    inner: &mut Inner,
    slice: &PersistableDaliSettingsSlice,
) {
    let current = inner.dali_settings;
    inner.dali_settings = DaliSettingsRecord {
        dt8_auto_activation_repair: slice.dt8_auto_activation_repair,
        dt8_rgbwaf_control_assert: slice.dt8_rgbwaf_control_assert,
        application_active: slice.application_active,
        device_short_address: slice.device_short_address,
        random_address: slice.random_address,
        device_groups: slice.device_groups,
        operating_mode: slice.operating_mode,
        power_cycle_notification: slice.power_cycle_notification,
        bus_unit_revision: current.bus_unit_revision.wrapping_add(1),
        wire_acknowledged: current.wire_acknowledged,
        wire_applied: current.wire_applied,
        operator_moved_at: current.operator_moved_at,
        quiescent_until: current.quiescent_until,
        quiescence_acknowledged: current.quiescence_acknowledged,
        ..DaliSettingsRecord::default()
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WireApplied {
    pub record: DaliSettingsRecord,
    pub nvm_changed: bool,
    pub application_active_moved: bool,
}

impl RegistryStore {
    // IEC 62386-103 §9.2.3
    pub(crate) fn apply_bus_unit_evidence(&self, body: &BusUnitNvmObservedEvent) -> Option<WireApplied> {
        let mut g = self.write_inner();
        let row = &mut g.dali_settings;
        if !is_newer(body.wire_sequence, row.wire_acknowledged) {
            return None;
        }
        let was_active = row.application_active;
        let nvm_changed = row.apply_wire(body);
        let record = *row;
        drop(g);
        if nvm_changed {
            self.dirty.mark_dali_settings_dirty();
        }
        self.publish_bus_unit();
        Some(WireApplied {
            record,
            nvm_changed,
            application_active_moved: record.application_active != was_active,
        })
    }
}

#[cfg(test)]
mod bus_unit_evidence_tests {
    use dali2rust_bsp::slice_store_files::InMemorySliceStore;
    use dali2rust_platform::slice_store::{SliceKey, SliceStore};

    use dali2rust_domain::dali::dev103::bus_unit::QUIESCENT_MS;

    use super::*;
    use crate::runtime::registry::persistence::validate_registry_slice;
    use crate::runtime::registry::persistence_slices::{encode_persistence_blob, PersistenceEnvelope};

    const FACTORY: BusUnitNvmRow = BusUnitNvmRow {
        short_address: None,
        random_address: FACTORY_RANDOM_ADDRESS,
        device_groups: 0,
        operating_mode: STANDARD_OPERATING_MODE,
        power_cycle_notification: false,
        application_active: true,
    };

    fn evidence(adopted: BusUnitNvmRow, observed: BusUnitNvmRow, wire_sequence: u32) -> BusUnitNvmObservedEvent {
        adopted_at(0, adopted, observed, wire_sequence)
    }

    fn adopted_at(
        adopted_revision: u32,
        adopted: BusUnitNvmRow,
        observed: BusUnitNvmRow,
        wire_sequence: u32,
    ) -> BusUnitNvmObservedEvent {
        BusUnitNvmObservedEvent { adopted, adopted_revision, observed, wire_sequence }
    }

    fn at(short_address: u8) -> BusUnitNvmRow {
        BusUnitNvmRow { short_address: Some(short_address), ..FACTORY }
    }

    fn row(store: &RegistryStore) -> DaliSettingsRecord {
        store.read_inner().dali_settings
    }

    #[test]
    fn a_wire_change_lands_acknowledged_with_a_new_revision() {
        let store = RegistryStore::with_adapter_count(1);
        let applied = store.apply_bus_unit_evidence(&evidence(FACTORY, at(5), 1)).expect("newer");
        assert!(applied.nvm_changed);
        assert!(!applied.application_active_moved);
        assert_eq!(row(&store).device_short_address, Some(5));
        assert_eq!(row(&store).wire_acknowledged, 1);
        assert_eq!(row(&store).bus_unit_revision, 1);
        assert!(store.dirty.take_dali_settings_dirty(), "a moved NVM value is on its way to flash");
    }

    #[test]
    fn only_the_fields_the_wire_moved_replace_the_operators() {
        let store = RegistryStore::with_adapter_count(1);
        store.write_inner().dali_settings.device_short_address = Some(9);
        let observed = BusUnitNvmRow { device_groups: 0b101, ..FACTORY };
        store.apply_bus_unit_evidence(&evidence(FACTORY, observed, 1)).expect("newer");
        assert_eq!(row(&store).device_short_address, Some(9), "the wire never moved the address");
        assert_eq!(row(&store).device_groups, 0b101);
    }

    #[test]
    fn a_cumulative_report_does_not_undo_an_operator_change_made_between_two_of_its_frames() {
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_evidence(&evidence(FACTORY, at(5), 1)).expect("first");
        store.write_inner().dali_settings.device_short_address = Some(9);
        let second = BusUnitNvmRow { device_groups: 1, ..at(5) };
        store.apply_bus_unit_evidence(&evidence(FACTORY, second, 2)).expect("second");
        assert_eq!(row(&store).device_short_address, Some(9), "the wire set 5 once and was answered");
        assert_eq!(row(&store).device_groups, 1);
    }

    #[test]
    fn the_same_value_from_a_new_adoption_is_applied_again() {
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_evidence(&evidence(FACTORY, at(5), 1)).expect("first");
        store.write_inner().dali_settings.device_short_address = Some(9);
        store.apply_bus_unit_evidence(&adopted_at(2, at(9), at(5), 2)).expect("second");
        assert_eq!(row(&store).device_short_address, Some(5), "the wire moved 9 to 5 after adopting 9");
    }

    #[test]
    fn a_wire_that_puts_a_value_back_before_it_is_answered_is_followed() {
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_evidence(&evidence(at(5), at(9), 1)).expect("first");
        store.apply_bus_unit_evidence(&evidence(at(5), at(5), 2)).expect("second");
        assert_eq!(row(&store).device_short_address, Some(5), "the wire set 9 and then 5 again");
        let passive = BusUnitNvmRow { application_active: false, ..FACTORY };
        store.apply_bus_unit_evidence(&evidence(FACTORY, passive, 3)).expect("disable");
        store.apply_bus_unit_evidence(&evidence(FACTORY, FACTORY, 4)).expect("enable");
        assert!(row(&store).application_active, "DISABLE then ENABLE leaves the unit active");
        assert_eq!(row(&store).application_active_moved_by, ApplicationActiveMover::Wire);
    }

    #[test]
    fn a_new_adoption_of_the_old_values_is_a_new_base() {
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_evidence(&evidence(at(5), at(9), 1)).expect("first");
        store.write_inner().dali_settings.device_short_address = Some(5);
        store.apply_bus_unit_evidence(&adopted_at(2, at(5), at(9), 2)).expect("second");
        assert_eq!(
            row(&store).device_short_address,
            Some(9),
            "the operator put 5 back, the unit adopted it, and the wire set 9 again"
        );
    }

    #[test]
    fn an_old_or_repeated_report_is_ignored() {
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_evidence(&evidence(FACTORY, at(5), 2)).expect("newer");
        assert!(store.apply_bus_unit_evidence(&evidence(FACTORY, at(6), 2)).is_none());
        assert!(store.apply_bus_unit_evidence(&evidence(FACTORY, at(7), 1)).is_none());
        assert_eq!(row(&store).device_short_address, Some(5));
        assert_eq!(row(&store).wire_acknowledged, 2);
    }

    #[test]
    fn a_wire_moved_application_active_says_so() {
        let store = RegistryStore::with_adapter_count(1);
        let passive = BusUnitNvmRow { application_active: false, ..FACTORY };
        let applied = store.apply_bus_unit_evidence(&evidence(FACTORY, passive, 1)).expect("newer");
        assert!(applied.application_active_moved);
        assert!(!row(&store).application_active);
        assert_eq!(row(&store).application_active_moved_by, ApplicationActiveMover::Wire);
    }

    #[test]
    fn a_report_that_moves_nothing_still_acknowledges_and_writes_nothing() {
        let store = RegistryStore::with_adapter_count(1);
        let applied = store.apply_bus_unit_evidence(&evidence(at(5), at(5), 1)).expect("newer");
        assert!(!applied.nvm_changed);
        assert_eq!(row(&store).wire_acknowledged, 1);
        assert!(!store.dirty.take_dali_settings_dirty());
    }

    fn published(sink: &AnswerInputs) -> BusUnitSnapshot {
        sink.bus_unit().expect("the registry has published its bus unit")
    }

    #[test]
    fn the_registry_publishes_its_bus_unit_on_install_and_after_every_change() {
        let store = RegistryStore::with_adapter_count(1);
        let sink = std::sync::Arc::new(AnswerInputs::new());
        store.install_bus_unit_sink(std::sync::Arc::clone(&sink));
        assert_eq!(published(&sink).nvm.short_address, None);
        store.apply_bus_unit_evidence(&evidence(FACTORY, at(5), 1)).expect("newer");
        assert_eq!(published(&sink).nvm.short_address, Some(5));
        assert_eq!(published(&sink).acknowledged_wire_sequence, 1);
        let patch = DaliSettingsUpdateCommand {
            patch_mask: DaliSettingsUpdateCommand::PATCH_DEVICE_SHORT_ADDRESS,
            dt8_auto_activation_repair: true,
            dt8_rgbwaf_control_assert: true,
            application_active: true,
            device_short_address: 9,
        };
        store.apply_dali_settings_from_command(&patch);
        assert_eq!(published(&sink), row(&store).bus_unit_snapshot(), "before the command is answered");
        assert_eq!(published(&sink).nvm.short_address, Some(9));
    }

    #[test]
    fn every_bus_unit_field_reaches_the_snapshot_from_the_wire() {
        let store = RegistryStore::with_adapter_count(1);
        let sink = std::sync::Arc::new(AnswerInputs::new());
        store.install_bus_unit_sink(std::sync::Arc::clone(&sink));
        let observed = BusUnitNvmRow {
            short_address: Some(17),
            random_address: 0x00AB_CDEF,
            device_groups: 0x8000_0001,
            operating_mode: 0x80,
            power_cycle_notification: true,
            application_active: false,
        };
        store.apply_bus_unit_evidence(&evidence(FACTORY, observed, 1)).expect("newer");
        let nvm = published(&sink).nvm;
        assert_eq!(
            (nvm.short_address, nvm.random_address, nvm.device_groups, nvm.operating_mode),
            (Some(17), 0x00AB_CDEF, 0x8000_0001, 0x80)
        );
        assert!(nvm.power_cycle_notification && !nvm.application_active);
    }

    #[test]
    fn a_slice_stored_before_the_bus_unit_keeps_its_values_and_starts_the_unit_from_factory() {
        let before = DaliSettingsBeforeBusUnitSlice {
            dt8_auto_activation_repair: false,
            dt8_rgbwaf_control_assert: false,
            application_active: false,
            device_short_address: Some(9),
        };
        let envelope = PersistenceEnvelope::new(DALI_SETTINGS_SLICE_VERSION, before);
        let bytes = encode_persistence_blob(&envelope).expect("encodes");
        let slices = InMemorySliceStore::default();
        let mut session = slices.begin_write(SliceKey::DaliSettings).expect("writable");
        session.append(&bytes).expect("appends");
        session.commit().expect("commits");
        let store = RegistryStore::with_adapter_count(1);
        store.hydrate_from_store(&slices, 1);
        let after = row(&store);
        let kept = (after.dt8_auto_activation_repair, after.dt8_rgbwaf_control_assert);
        assert_eq!(kept, (false, false));
        assert_eq!((after.application_active, after.device_short_address), (false, Some(9)));
        let factory = DaliSettingsRecord::default();
        assert_eq!(
            (after.random_address, after.device_groups, after.operating_mode, after.power_cycle_notification),
            (factory.random_address, factory.device_groups, factory.operating_mode, factory.power_cycle_notification)
        );
        let validated = validate_registry_slice(SliceKey::DaliSettings, &bytes);
        assert!(matches!(validated, Some(Ok(()))), "{validated:?}");
    }

    const NOW_MS: u32 = 5_000;

    fn instant_at(ms: u32) -> Instant {
        static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        *BASE.get_or_init(Instant::now) + Duration::from_millis(u64::from(ms))
    }

    fn quiescence(sequence: u32, remaining_ms: Option<u32>) -> BusUnitQuiescenceObservedEvent {
        BusUnitQuiescenceObservedEvent { sequence, remaining_ms }
    }

    #[test]
    fn another_controller_s_quiescence_holds_until_its_deadline_and_is_acknowledged() {
        let store = RegistryStore::with_adapter_count(1);
        let sink = std::sync::Arc::new(AnswerInputs::new());
        store.install_bus_unit_sink(std::sync::Arc::clone(&sink));
        assert!(store.apply_bus_unit_quiescence(&quiescence(1, Some(QUIESCENT_MS)), instant_at(NOW_MS)));
        assert_eq!(published(&sink).acknowledged_quiescence, 1);
        assert_eq!(store.quiescent_remaining_at(instant_at(NOW_MS + 1)), Some(QUIESCENT_MS - 1));
        assert_eq!(store.quiescent_remaining_at(instant_at(NOW_MS + QUIESCENT_MS - 1)), Some(1));
        assert_eq!(store.quiescent_remaining_at(instant_at(NOW_MS + QUIESCENT_MS)), None, "it lapses without a stop");
        assert!(!store.dirty.take_dali_settings_dirty(), "quiescence is never persisted");
    }

    #[test]
    fn a_resend_moves_the_deadline_to_what_it_says_is_left_and_reports_no_change() {
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_quiescence(&quiescence(1, Some(QUIESCENT_MS)), instant_at(NOW_MS));
        let later = NOW_MS + 1_000;
        assert!(!store.apply_bus_unit_quiescence(&quiescence(1, Some(QUIESCENT_MS - 2_000)), instant_at(later)));
        assert_eq!(store.quiescent_remaining_at(instant_at(later)), Some(QUIESCENT_MS - 2_000));
    }

    #[test]
    fn a_stop_ends_quiescence_and_an_older_report_arriving_after_it_does_not_restart_it() {
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_quiescence(&quiescence(1, Some(QUIESCENT_MS)), instant_at(NOW_MS));
        assert!(store.apply_bus_unit_quiescence(&quiescence(2, None), instant_at(NOW_MS + 1)));
        assert_eq!(store.quiescent_remaining_at(instant_at(NOW_MS + 2)), None);
        assert!(!store.apply_bus_unit_quiescence(&quiescence(1, Some(QUIESCENT_MS)), instant_at(NOW_MS + 3)));
        assert_eq!(store.quiescent_remaining_at(instant_at(NOW_MS + 4)), None);
        assert_eq!(row(&store).quiescence_acknowledged, 2);
    }

    #[test]
    fn a_slice_read_again_keeps_another_controller_s_quiescence() {
        let stored = RegistryStore::with_adapter_count(1);
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_quiescence(&quiescence(4, Some(QUIESCENT_MS)), instant_at(NOW_MS));
        store.hydrate_from_store(&stored_slice(&stored), 1);
        assert_eq!(store.quiescent_remaining_at(instant_at(NOW_MS)), Some(QUIESCENT_MS));
        assert_eq!(row(&store).bus_unit_snapshot().acknowledged_quiescence, 4);
    }

    #[test]
    fn a_lapsed_quiescence_stays_lapsed_however_long_nothing_is_heard() {
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_quiescence(&quiescence(1, Some(QUIESCENT_MS)), instant_at(NOW_MS));
        let a_clock_wrap_later = instant_at(NOW_MS + 1) + Duration::from_millis(1 << 32);
        assert_eq!(store.quiescent_remaining_at(a_clock_wrap_later), None);
        let half_a_wrap_later = instant_at(NOW_MS + QUIESCENT_MS) + Duration::from_millis(1 << 31);
        assert_eq!(store.quiescent_remaining_at(half_a_wrap_later), None);
    }

    fn stored_slice(store: &RegistryStore) -> InMemorySliceStore {
        let bytes = encode_persistence_blob(&PersistenceEnvelope::new(
            DALI_SETTINGS_SLICE_VERSION,
            persistable_snapshot(&store.read_inner()),
        ))
        .expect("encodes");
        let slices = InMemorySliceStore::default();
        let mut session = slices.begin_write(SliceKey::DaliSettings).expect("writable");
        session.append(&bytes).expect("appends");
        session.commit().expect("commits");
        slices
    }

    #[test]
    fn hydration_hands_the_unit_the_stored_copy_before_anything_else_moves_it() {
        let stored = RegistryStore::with_adapter_count(1);
        stored.apply_bus_unit_evidence(&evidence(FACTORY, at(7), 1)).expect("newer");
        let slices = stored_slice(&stored);
        let store = RegistryStore::with_adapter_count(1);
        let sink = std::sync::Arc::new(AnswerInputs::new());
        store.install_bus_unit_sink(std::sync::Arc::clone(&sink));
        let before = sink.bus_unit().expect("the install publishes");
        store.hydrate_from_store(&slices, 1);
        let after = sink.bus_unit().expect("still published");
        assert_eq!(after.nvm.short_address, Some(7));
        assert!(is_newer(after.revision, before.revision), "the unit adopts the stored copy");
    }

    #[test]
    fn a_stored_slice_brings_back_every_variable_the_wire_moved() {
        let stored = RegistryStore::with_adapter_count(1);
        let moved = BusUnitNvmRow {
            random_address: 0x00AB_CDEF,
            device_groups: 0x0000_0101,
            operating_mode: 0x80,
            power_cycle_notification: true,
            application_active: false,
            ..at(7)
        };
        stored.apply_bus_unit_evidence(&evidence(FACTORY, moved, 1)).expect("newer");
        let store = RegistryStore::with_adapter_count(1);
        store.hydrate_from_store(&stored_slice(&stored), 1);
        assert_eq!(row(&store).bus_unit_nvm(), moved);
    }

    #[test]
    fn an_older_image_reads_the_slice_this_one_writes_and_keeps_its_role() {
        let store = RegistryStore::with_adapter_count(1);
        let passive = BusUnitNvmRow { application_active: false, ..at(9) };
        store.apply_bus_unit_evidence(&evidence(FACTORY, passive, 1)).expect("newer");
        let written = persistable_snapshot(&store.read_inner());
        let bytes = encode_persistence_blob(&PersistenceEnvelope::new(DALI_SETTINGS_SLICE_VERSION, written))
            .expect("encodes");
        let older: DaliSettingsBeforeBusUnitSlice =
            decode_versioned_slice(&bytes, DALI_SETTINGS_SLICE_VERSION).expect("the prefix decodes");
        assert_eq!((older.application_active, older.device_short_address), (false, Some(9)));
        assert_eq!(
            (older.dt8_auto_activation_repair, older.dt8_rgbwaf_control_assert),
            (written.dt8_auto_activation_repair, written.dt8_rgbwaf_control_assert)
        );
    }

    #[test]
    fn a_reload_keeps_the_wire_acknowledgement_and_moves_the_revision() {
        let store = RegistryStore::with_adapter_count(1);
        store.apply_bus_unit_evidence(&evidence(FACTORY, at(5), 3)).expect("newer");
        let before = row(&store);
        let mut slice = persistable_snapshot(&store.read_inner());
        slice.device_short_address = Some(7);
        hydrate_dali_settings_inner(&mut store.write_inner(), &slice);
        let after = row(&store);
        assert_eq!(after.device_short_address, Some(7));
        assert_eq!(after.wire_acknowledged, before.wire_acknowledged, "the unit's reports stay answered");
        assert!(is_newer(after.bus_unit_revision, before.bus_unit_revision), "the unit adopts the reload");
    }

    #[test]
    fn an_operator_change_to_a_bus_unit_value_moves_the_revision() {
        let store = RegistryStore::with_adapter_count(1);
        let mut patch = DaliSettingsUpdateCommand {
            patch_mask: DaliSettingsUpdateCommand::PATCH_DEVICE_SHORT_ADDRESS,
            dt8_auto_activation_repair: true,
            dt8_rgbwaf_control_assert: true,
            application_active: true,
            device_short_address: 9,
        };
        store.apply_dali_settings_from_command(&patch);
        assert_eq!(row(&store).bus_unit_revision, 1);
        patch.patch_mask = DaliSettingsUpdateCommand::PATCH_DT8_AUTO_ACTIVATION_REPAIR;
        patch.dt8_auto_activation_repair = false;
        store.apply_dali_settings_from_command(&patch);
        assert_eq!(row(&store).bus_unit_revision, 1, "a DT8 permission is not a bus-unit value");
    }

    fn operator_short_address(store: &RegistryStore, short_address: u8) -> u32 {
        store.apply_dali_settings_from_command(&DaliSettingsUpdateCommand {
            patch_mask: DaliSettingsUpdateCommand::PATCH_DEVICE_SHORT_ADDRESS,
            dt8_auto_activation_repair: true,
            dt8_rgbwaf_control_assert: true,
            application_active: true,
            device_short_address: short_address,
        });
        row(store).bus_unit_revision
    }

    #[test]
    fn an_operator_value_holds_against_a_wire_change_made_before_the_unit_adopted_it() {
        let store = RegistryStore::with_adapter_count(1);
        let adopted = row(&store).bus_unit_revision;
        operator_short_address(&store, 9);
        let wire = BusUnitNvmRow { device_groups: 0x0000_0101, ..at(5) };
        store.apply_bus_unit_evidence(&adopted_at(adopted, FACTORY, wire, 1)).expect("newer");
        let after = row(&store);
        assert_eq!(after.device_short_address, Some(9), "the confirmed PATCH stands");
        assert_eq!(after.device_groups, 0x0000_0101, "a field the operator did not touch follows the wire");
        assert_eq!(after.wire_acknowledged, 1, "the report is answered all the same");
    }

    #[test]
    fn an_operator_value_still_holds_after_the_slice_is_read_again() {
        let store = RegistryStore::with_adapter_count(1);
        let adopted = row(&store).bus_unit_revision;
        operator_short_address(&store, 9);
        store.hydrate_from_store(&stored_slice(&store), 1);
        store.apply_bus_unit_evidence(&adopted_at(adopted, FACTORY, at(5), 1)).expect("newer");
        assert_eq!(row(&store).device_short_address, Some(9));
    }

    #[test]
    fn a_wire_change_made_on_a_copy_newer_than_the_operator_value_replaces_it() {
        let store = RegistryStore::with_adapter_count(1);
        let operator = operator_short_address(&store, 9);
        let grouped = BusUnitNvmRow { device_groups: 1, ..at(9) };
        store.apply_bus_unit_evidence(&adopted_at(operator, at(9), grouped, 1)).expect("newer");
        let later = row(&store).bus_unit_revision;
        assert!(is_newer(later, operator));
        let readdressed = BusUnitNvmRow { short_address: Some(5), ..grouped };
        store.apply_bus_unit_evidence(&adopted_at(later, grouped, readdressed, 2)).expect("newer");
        assert_eq!(row(&store).device_short_address, Some(5));
    }

    #[test]
    fn a_wire_change_made_after_the_unit_adopted_the_operator_value_replaces_it() {
        let store = RegistryStore::with_adapter_count(1);
        let adopted = row(&store).bus_unit_revision;
        store.apply_bus_unit_evidence(&adopted_at(adopted, FACTORY, at(5), 1)).expect("newer");
        let operator = operator_short_address(&store, 9);
        store.apply_bus_unit_evidence(&adopted_at(operator, at(9), at(5), 3)).expect("newer");
        assert_eq!(row(&store).device_short_address, Some(5));
    }
}
