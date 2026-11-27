use dali2rust_contracts::msg::{
    ClusterForwarding, ClusterSettingsField as Field, ClusterSettingsUpdateCommand,
    ClusterTextField, ClusterTextUpdateCommand, ErrorCode,
};
use dali2rust_domain::registry::{
    ClusterSettingsConflict, ClusterSettingsReadPort, ClusterSettingsView, ClusterWire,
};
use dali2rust_platform::slice_store::StoreError;

use super::persistence_slices::{
    decode_versioned_slice, PersistableClusterSettingsSlice, CLUSTER_SETTINGS_SLICE_VERSION,
};
use super::store::{registry_unix_ms, Inner, RegistryStore, STAGE_MAX_AGE_MS};

const MAX_CLUSTER_STAGED_FRAMES: usize = 2;

pub(crate) const CLUSTER_STAGE_MAX_AGE_MS: u64 = STAGE_MAX_AGE_MS;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ClusterSettingsRecord(pub ClusterSettingsView);

pub(crate) enum ClusterPatch {
    Settings(ClusterSettingsUpdateCommand),
    Text(ClusterTextUpdateCommand),
}

pub(crate) struct ClusterStage {
    series: u64,
    patches: Vec<ClusterPatch>,
    staged_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClusterRefusal {
    Standby,
    TooManyFrames,
    NoStage,
    FrameCountMismatch,
    OutOfRange,
    Conflict(ClusterSettingsConflict),
}

impl ClusterRefusal {
    pub(crate) const fn code(self) -> ErrorCode {
        match self {
            Self::Standby | Self::Conflict(_) => ErrorCode::Conflict,
            Self::TooManyFrames | Self::NoStage | Self::FrameCountMismatch | Self::OutOfRange => {
                ErrorCode::InvalidValue
            }
        }
    }

    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Standby => "controller_passive",
            Self::TooManyFrames => "cluster_stage_limit",
            Self::NoStage => "cluster_no_stage",
            Self::FrameCountMismatch => "cluster_frame_mismatch",
            Self::OutOfRange => "cluster_out_of_range",
            Self::Conflict(conflict) => conflict.message(),
        }
    }
}

fn set<T>(slot: &mut T, value: T, patched: bool) {
    if patched {
        *slot = value;
    }
}

fn apply_settings(view: &mut ClusterSettingsView, body: &ClusterSettingsUpdateCommand) {
    let on = |field| body.patches(field);
    let wire = ClusterWire::from_same_as(body.wire_same_as);
    set(&mut view.enabled, body.enabled, on(Field::Enabled));
    set(&mut view.node_number, body.node_number, on(Field::NodeNumber));
    set(&mut view.port, body.port, on(Field::Port));
    set(&mut view.wire, wire, on(Field::Wire));
    apply_forwarding(&mut view.forwarding, body);
    set(&mut view.accept_all_lamps, body.accept_all_lamps, on(Field::AcceptAllLamps));
    set(&mut view.send_all_lamps, body.send_all_lamps, on(Field::SendAllLamps));
    set(&mut view.accept_single_lamps, body.accept_single_lamps, on(Field::AcceptSingleLamps));
    set(&mut view.wire_protection, body.wire_protection, on(Field::WireProtection));
}

fn apply_forwarding(forwarding: &mut ClusterForwarding, body: &ClusterSettingsUpdateCommand) {
    let on = |field| body.patches(field);
    let to = body.forwarding;
    set(&mut forwarding.operator, to.operator, on(Field::ForwardOperator));
    set(&mut forwarding.home_assistant, to.home_assistant, on(Field::ForwardHomeAssistant));
    set(&mut forwarding.hcl, to.hcl, on(Field::ForwardHcl));
    set(&mut forwarding.foreign_master, to.foreign_master, on(Field::ForwardForeignMaster));
}

impl ClusterPatch {
    fn apply(&self, view: &mut ClusterSettingsView) {
        match self {
            Self::Settings(body) => apply_settings(view, body),
            Self::Text(body) => match body.field {
                ClusterTextField::ClusterName => view.cluster_name = body.text.as_str().to_string(),
            },
        }
    }
}

fn checked(view: ClusterSettingsView) -> Result<ClusterSettingsView, ClusterRefusal> {
    if !view.in_range() {
        return Err(ClusterRefusal::OutOfRange);
    }
    view.conflict().map_or(Ok(view), |conflict| Err(ClusterRefusal::Conflict(conflict)))
}

fn merged(stored: &ClusterSettingsView, patches: &[ClusterPatch]) -> Result<ClusterSettingsView, ClusterRefusal> {
    let mut view = stored.clone();
    for patch in patches {
        patch.apply(&mut view);
    }
    checked(view)
}

impl RegistryStore {
    pub(crate) fn stage_cluster_patch(&self, series: u64, patch: ClusterPatch) -> Result<(), ClusterRefusal> {
        let mut g = self.write_inner();
        let slot = &mut g.cluster_stage;
        slot.take_if(|stage| stage.series != series);
        let stage = slot.get_or_insert_with(|| ClusterStage {
            series,
            patches: Vec::with_capacity(MAX_CLUSTER_STAGED_FRAMES),
            staged_at_ms: registry_unix_ms(),
        });
        if stage.patches.len() >= MAX_CLUSTER_STAGED_FRAMES {
            *slot = None;
            return Err(ClusterRefusal::TooManyFrames);
        }
        stage.patches.push(patch);
        Ok(())
    }

    pub(crate) fn commit_cluster_settings(
        &self,
        series: u64,
        frames: u8,
    ) -> Result<ClusterSettingsRecord, ClusterRefusal> {
        let mut g = self.write_inner();
        let stage = g.cluster_stage.take_if(|stage| stage.series == series).ok_or(ClusterRefusal::NoStage)?;
        if !g.dali_settings.application_active {
            return Err(ClusterRefusal::Standby);
        }
        if stage.patches.len() != usize::from(frames) {
            return Err(ClusterRefusal::FrameCountMismatch);
        }
        g.cluster_settings = ClusterSettingsRecord(merged(&g.cluster_settings.0, &stage.patches)?);
        let applied = g.cluster_settings.clone();
        drop(g);
        self.dirty.mark_cluster_settings_dirty();
        Ok(applied)
    }

    pub(crate) fn evict_stale_cluster_stage(&self, max_age_ms: u64) {
        let now = registry_unix_ms();
        self.write_inner()
            .cluster_stage
            .take_if(|stage| now.saturating_sub(stage.staged_at_ms) > max_age_ms);
    }
}

impl ClusterSettingsReadPort for RegistryStore {
    fn cluster_settings_view(&self) -> ClusterSettingsView {
        self.read_inner().cluster_settings.0.clone()
    }
}

pub(crate) fn persistable_snapshot(inner: &Inner) -> PersistableClusterSettingsSlice {
    let row = &inner.cluster_settings.0;
    PersistableClusterSettingsSlice {
        enabled: row.enabled,
        cluster_name: row.cluster_name.clone(),
        node_number: row.node_number,
        port: row.port,
        wire_same_as: row.wire.same_as(),
        forward_operator: row.forwarding.operator,
        forward_home_assistant: row.forwarding.home_assistant,
        forward_hcl: row.forwarding.hcl,
        forward_foreign_master: row.forwarding.foreign_master,
        accept_all_lamps: row.accept_all_lamps,
        send_all_lamps: row.send_all_lamps,
        accept_single_lamps: row.accept_single_lamps,
        wire_protection: row.wire_protection,
    }
}

fn stored_view(slice: &PersistableClusterSettingsSlice) -> ClusterSettingsView {
    ClusterSettingsView {
        enabled: slice.enabled,
        cluster_name: slice.cluster_name.clone(),
        node_number: slice.node_number,
        port: slice.port,
        wire: ClusterWire::from_same_as(slice.wire_same_as),
        forwarding: ClusterForwarding {
            operator: slice.forward_operator,
            home_assistant: slice.forward_home_assistant,
            hcl: slice.forward_hcl,
            foreign_master: slice.forward_foreign_master,
        },
        accept_all_lamps: slice.accept_all_lamps,
        send_all_lamps: slice.send_all_lamps,
        accept_single_lamps: slice.accept_single_lamps,
        wire_protection: slice.wire_protection,
    }
}

pub(crate) fn decode_cluster_settings(bytes: &[u8]) -> Result<PersistableClusterSettingsSlice, StoreError> {
    let slice = decode_versioned_slice::<PersistableClusterSettingsSlice>(bytes, CLUSTER_SETTINGS_SLICE_VERSION)
        .map_err(|e| StoreError::Backend(e.to_string()))?;
    checked(stored_view(&slice))
        .map(|_| slice)
        .map_err(|refusal| StoreError::Backend(format!("cluster settings refused: {}", refusal.message())))
}

pub(crate) fn hydrate_cluster_settings_inner(inner: &mut Inner, slice: &PersistableClusterSettingsSlice) {
    inner.cluster_settings = ClusterSettingsRecord(stored_view(slice));
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::fixed_text_64;
    use dali2rust_domain::registry::{CLUSTER_NODE_COUNT, MAX_CLUSTER_NAME_BYTES};

    use super::super::persistence_slices::encode_persistence_blob;
    use super::super::persistence_slices::PersistenceEnvelope;
    use super::*;

    const SERIES: u64 = 7;

    fn settings(fields: &[Field]) -> ClusterSettingsUpdateCommand {
        let mut body = ClusterSettingsUpdateCommand {
            series: SERIES,
            patch_mask: 0,
            enabled: true,
            node_number: 3,
            port: 47_900,
            wire_same_as: Some(1),
            forwarding: ClusterForwarding { operator: false, home_assistant: false, hcl: true, foreign_master: true },
            accept_all_lamps: true,
            send_all_lamps: true,
            accept_single_lamps: true,
            wire_protection: true,
        };
        for field in fields {
            body.patch(*field);
        }
        body
    }

    fn name(text: &str) -> ClusterPatch {
        ClusterPatch::Text(ClusterTextUpdateCommand {
            series: SERIES,
            field: ClusterTextField::ClusterName,
            text: fixed_text_64(text),
        })
    }

    fn commit(store: &RegistryStore, patches: Vec<ClusterPatch>) -> Result<ClusterSettingsRecord, ClusterRefusal> {
        let frames = u8::try_from(patches.len()).expect("frames");
        for patch in patches {
            store.stage_cluster_patch(SERIES, patch)?;
        }
        store.commit_cluster_settings(SERIES, frames)
    }

    #[test]
    fn a_patch_moves_only_the_fields_its_mask_names() {
        let store = RegistryStore::with_adapter_count(1);
        let applied = commit(&store, vec![ClusterPatch::Settings(settings(&[Field::Port, Field::ForwardHcl]))]);
        let mut expected = ClusterSettingsView { port: 47_900, ..ClusterSettingsView::default() };
        expected.forwarding.hcl = true;
        assert_eq!(applied.expect("applied").0, expected);
    }

    fn moved(before: &ClusterSettingsView, after: &ClusterSettingsView) -> Vec<Field> {
        let f = (&before.forwarding, &after.forwarding);
        [
            (Field::Enabled, before.enabled != after.enabled),
            (Field::NodeNumber, before.node_number != after.node_number),
            (Field::Port, before.port != after.port),
            (Field::Wire, before.wire != after.wire),
            (Field::ForwardOperator, f.0.operator != f.1.operator),
            (Field::ForwardHomeAssistant, f.0.home_assistant != f.1.home_assistant),
            (Field::ForwardHcl, f.0.hcl != f.1.hcl),
            (Field::ForwardForeignMaster, f.0.foreign_master != f.1.foreign_master),
            (Field::AcceptAllLamps, before.accept_all_lamps != after.accept_all_lamps),
            (Field::SendAllLamps, before.send_all_lamps != after.send_all_lamps),
            (Field::AcceptSingleLamps, before.accept_single_lamps != after.accept_single_lamps),
            (Field::WireProtection, before.wire_protection != after.wire_protection),
        ]
        .into_iter()
        .filter_map(|(field, changed)| changed.then_some(field))
        .collect()
    }

    type Flip = fn(&mut ClusterSettingsUpdateCommand);

    const FLIPS: [(Field, Flip); 12] = [
        (Field::Enabled, |b| b.enabled = true),
        (Field::NodeNumber, |b| b.node_number = 3),
        (Field::Port, |b| b.port = 47_900),
        (Field::Wire, |b| b.wire_same_as = Some(1)),
        (Field::ForwardOperator, |b| b.forwarding.operator = false),
        (Field::ForwardHomeAssistant, |b| b.forwarding.home_assistant = false),
        (Field::ForwardHcl, |b| b.forwarding.hcl = true),
        (Field::ForwardForeignMaster, |b| b.forwarding.foreign_master = true),
        (Field::AcceptAllLamps, |b| b.accept_all_lamps = true),
        (Field::SendAllLamps, |b| b.send_all_lamps = true),
        (Field::AcceptSingleLamps, |b| b.accept_single_lamps = true),
        (Field::WireProtection, |b| b.wire_protection = false),
    ];

    fn factory_body() -> ClusterSettingsUpdateCommand {
        let factory = ClusterSettingsView::default();
        ClusterSettingsUpdateCommand {
            series: SERIES,
            patch_mask: 0,
            enabled: factory.enabled,
            node_number: factory.node_number,
            port: factory.port,
            wire_same_as: factory.wire.same_as(),
            forwarding: factory.forwarding,
            accept_all_lamps: factory.accept_all_lamps,
            send_all_lamps: factory.send_all_lamps,
            accept_single_lamps: factory.accept_single_lamps,
            wire_protection: factory.wire_protection,
        }
    }

    #[test]
    fn each_field_the_mask_names_lands_in_its_own_place_from_its_own_value() {
        assert_eq!(FLIPS.map(|(field, _)| field), Field::ALL);
        let factory = ClusterSettingsView::default();
        for (field, flip) in FLIPS {
            let mut body = factory_body();
            flip(&mut body);
            body.patch(field);
            let mut view = factory.clone();
            apply_settings(&mut view, &body);
            assert_eq!(moved(&factory, &view), vec![field], "{field:?}");
        }
    }

    #[test]
    fn every_field_survives_the_stored_slice() {
        for (field, flip) in FLIPS {
            let mut body = factory_body();
            flip(&mut body);
            body.patch(field);
            let mut inner = Inner::default();
            apply_settings(&mut inner.cluster_settings.0, &body);
            inner.cluster_settings.0.cluster_name = "dali-cluster".to_string();
            let slice = persistable_snapshot(&inner);
            let mut restored = Inner::default();
            hydrate_cluster_settings_inner(&mut restored, &slice);
            assert_eq!(restored.cluster_settings, inner.cluster_settings, "{field:?}");
        }
    }

    #[test]
    fn an_unnamed_field_keeps_its_value_whatever_the_body_carries() {
        let factory = ClusterSettingsView::default();
        let mut view = factory.clone();
        apply_settings(&mut view, &settings(&[]));
        assert_eq!(view, factory);
    }

    #[test]
    fn the_name_frame_sets_the_name() {
        let mut view = ClusterSettingsView::default();
        name("dali-cluster").apply(&mut view);
        assert_eq!(view, ClusterSettingsView { cluster_name: "dali-cluster".to_string(), ..ClusterSettingsView::default() });
    }

    #[test]
    fn a_conflict_is_judged_on_the_merged_record_and_changes_nothing() {
        let store = RegistryStore::with_adapter_count(1);
        let shared = ClusterPatch::Settings(settings(&[Field::Wire, Field::ForwardForeignMaster]));
        commit(&store, vec![shared]).expect("a protected shared wire is fine");
        let before = store.cluster_settings_view();
        let mut unprotect = settings(&[Field::WireProtection]);
        unprotect.wire_protection = false;
        let refused = commit(&store, vec![ClusterPatch::Settings(unprotect)]);
        let storm = ClusterRefusal::Conflict(ClusterSettingsConflict::WireProtectionRequired);
        assert_eq!(refused, Err(storm));
        assert_eq!(storm.code(), ErrorCode::Conflict);
        assert_eq!(store.cluster_settings_view(), before);
    }

    #[test]
    fn a_value_out_of_range_is_refused_as_invalid() {
        let store = RegistryStore::with_adapter_count(1);
        let mut body = settings(&[Field::NodeNumber]);
        body.node_number = CLUSTER_NODE_COUNT;
        let refused = commit(&store, vec![ClusterPatch::Settings(body)]);
        assert_eq!(refused, Err(ClusterRefusal::OutOfRange));
        assert_eq!(ClusterRefusal::OutOfRange.code(), ErrorCode::InvalidValue);
        assert_eq!(store.cluster_settings_view(), ClusterSettingsView::default());
    }

    #[test]
    fn a_standby_refuses_the_commit_whenever_the_frames_were_staged() {
        let store = RegistryStore::with_adapter_count(1);
        store.stage_cluster_patch(SERIES, name("dali-cluster")).expect("staged while active");
        store.write_inner().dali_settings.application_active = false;
        assert_eq!(store.commit_cluster_settings(SERIES, 1), Err(ClusterRefusal::Standby));
        assert_eq!(commit(&store, vec![name("x")]), Err(ClusterRefusal::Standby));
        assert_eq!(ClusterRefusal::Standby.message(), "controller_passive");
        assert_eq!(store.cluster_settings_view(), ClusterSettingsView::default());
    }

    #[test]
    fn a_frame_count_that_disagrees_with_the_stage_is_refused() {
        let store = RegistryStore::with_adapter_count(1);
        store.stage_cluster_patch(SERIES, name("dali-cluster")).expect("staged");
        assert_eq!(store.commit_cluster_settings(SERIES, 2), Err(ClusterRefusal::FrameCountMismatch));
        assert_eq!(store.commit_cluster_settings(SERIES, 1), Err(ClusterRefusal::NoStage));
    }

    const STAGE_AGE_MS: u64 = 1_000;

    #[test]
    fn a_new_series_replaces_a_half_staged_one() {
        let store = RegistryStore::with_adapter_count(1);
        store.stage_cluster_patch(SERIES, name("abandoned")).expect("staged");
        store.stage_cluster_patch(SERIES + 1, name("current")).expect("staged");
        let applied = store.commit_cluster_settings(SERIES + 1, 1).expect("committed");
        assert_eq!(applied.0.cluster_name, "current");
    }

    #[test]
    fn a_commit_of_another_series_leaves_the_stage_alone() {
        let store = RegistryStore::with_adapter_count(1);
        store.stage_cluster_patch(SERIES, name("dali-cluster")).expect("staged");
        assert_eq!(store.commit_cluster_settings(SERIES + 1, 1), Err(ClusterRefusal::NoStage));
        assert_eq!(store.cluster_settings_view(), ClusterSettingsView::default());
        let applied = store.commit_cluster_settings(SERIES, 1).expect("committed");
        assert_eq!(applied.0.cluster_name, "dali-cluster");
    }

    #[test]
    fn a_frame_past_the_stage_limit_drops_the_stage() {
        let store = RegistryStore::with_adapter_count(1);
        for text in ["first", "second"] {
            store.stage_cluster_patch(SERIES, name(text)).expect("within the limit");
        }
        let third = store.stage_cluster_patch(SERIES, name("third"));
        assert_eq!(third, Err(ClusterRefusal::TooManyFrames));
        assert_eq!(store.commit_cluster_settings(SERIES, 2), Err(ClusterRefusal::NoStage));
        assert_eq!(store.cluster_settings_view(), ClusterSettingsView::default());
    }

    #[test]
    fn only_a_stage_older_than_the_limit_is_evicted() {
        let store = RegistryStore::with_adapter_count(1);
        store.stage_cluster_patch(SERIES, name("dali-cluster")).expect("staged");
        if let Some(stage) = store.write_inner().cluster_stage.as_mut() {
            stage.staged_at_ms = registry_unix_ms().saturating_sub(STAGE_AGE_MS);
        }
        store.evict_stale_cluster_stage(STAGE_AGE_MS * 2);
        assert!(store.read_inner().cluster_stage.is_some(), "a young stage stays");
        store.evict_stale_cluster_stage(STAGE_AGE_MS / 2);
        assert!(store.read_inner().cluster_stage.is_none(), "an old stage goes");
    }

    #[test]
    fn a_stored_slice_that_breaks_a_rule_does_not_decode() {
        let mut slice = persistable_snapshot(&Inner::default());
        slice.wire_same_as = Some(slice.node_number);
        let envelope = PersistenceEnvelope { version: CLUSTER_SETTINGS_SLICE_VERSION, data: slice.clone() };
        let bytes = encode_persistence_blob(&envelope).expect("encode");
        assert!(decode_cluster_settings(&bytes).is_err());
        slice.wire_same_as = None;
        let envelope = PersistenceEnvelope { version: CLUSTER_SETTINGS_SLICE_VERSION, data: slice.clone() };
        let bytes = encode_persistence_blob(&envelope).expect("encode");
        assert_eq!(decode_cluster_settings(&bytes).expect("decodes"), slice);
    }

    #[test]
    fn the_widest_slice_fits_one_small_bank() {
        let widest = PersistableClusterSettingsSlice {
            cluster_name: "\u{10FFFF}".repeat(MAX_CLUSTER_NAME_BYTES / 4),
            port: u16::MAX,
            wire_same_as: Some(u8::MAX),
            ..persistable_snapshot(&Inner::default())
        };
        let envelope = PersistenceEnvelope { version: u32::MAX, data: widest };
        let bytes = encode_persistence_blob(&envelope).expect("encode");
        let payload = dali2rust_bsp::slice_layout::SMALL_SLOT_PAYLOAD_BYTES;
        assert!(bytes.len() <= usize::try_from(payload).expect("payload"), "{} B", bytes.len());
    }
}
