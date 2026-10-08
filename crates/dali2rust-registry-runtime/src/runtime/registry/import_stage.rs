use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use dali2rust_platform::slice_store::SliceKey;

use dali2rust_contracts::msg::CONFIG_WRITE_TTL_MS;

use super::store::RegistryStore;

pub const IMPORT_STAGE_MAX_AGE: Duration = Duration::from_millis(CONFIG_WRITE_TTL_MS as u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedSlice {
    pub key: SliceKey,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStageRefusal {
    Busy,
}

struct PendingImport {
    workflow: u64,
    slices: Vec<StagedSlice>,
    staged_at: Instant,
}

#[derive(Default)]
pub(crate) struct ImportStage(Mutex<Option<PendingImport>>);

impl ImportStage {
    fn lock(&self) -> MutexGuard<'_, Option<PendingImport>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl RegistryStore {
    pub fn stage_import(
        &self,
        workflow: u64,
        slices: Vec<StagedSlice>,
    ) -> Result<(), ImportStageRefusal> {
        let mut stage = self.import_stage.lock();
        if stage
            .as_ref()
            .is_some_and(|pending| pending.staged_at.elapsed() < IMPORT_STAGE_MAX_AGE)
        {
            return Err(ImportStageRefusal::Busy);
        }
        *stage = Some(PendingImport {
            workflow,
            slices,
            staged_at: Instant::now(),
        });
        Ok(())
    }

    pub fn discard_import(&self, workflow: u64) {
        let mut stage = self.import_stage.lock();
        if stage.as_ref().is_some_and(|pending| pending.workflow == workflow) {
            *stage = None;
        }
    }

    pub(crate) fn take_import(&self, workflow: u64) -> Option<Vec<StagedSlice>> {
        let mut stage = self.import_stage.lock();
        if stage.as_ref().is_none_or(|pending| pending.workflow != workflow) {
            return None;
        }
        stage
            .take()
            .filter(|pending| pending.staged_at.elapsed() < IMPORT_STAGE_MAX_AGE)
            .map(|pending| pending.slices)
    }

    pub(crate) fn evict_stale_import(&self) {
        let mut stage = self.import_stage.lock();
        if stage
            .as_ref()
            .is_some_and(|pending| pending.staged_at.elapsed() >= IMPORT_STAGE_MAX_AGE)
        {
            *stage = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged() -> Vec<StagedSlice> {
        vec![StagedSlice {
            key: SliceKey::PollerSettings,
            bytes: vec![1, 2, 3],
        }]
    }

    #[test]
    fn a_second_import_is_refused_while_the_first_waits_for_its_commit() {
        let store = RegistryStore::with_adapter_count(1);
        store.stage_import(7, staged()).expect("first stage");
        assert_eq!(store.stage_import(8, staged()), Err(ImportStageRefusal::Busy));
        assert_eq!(store.take_import(8), None, "a commit takes only its own stage");
        assert_eq!(store.take_import(7), Some(staged()));
        store.stage_import(8, staged()).expect("the stage is free once taken");
    }

    #[test]
    fn a_stage_nobody_commits_gives_way_once_it_is_stale() {
        let store = RegistryStore::with_adapter_count(1);
        let age = |store: &RegistryStore| {
            if let Some(pending) = store.import_stage.lock().as_mut() {
                pending.staged_at = Instant::now().checked_sub(IMPORT_STAGE_MAX_AGE).expect("clock");
            }
        };
        store.stage_import(7, staged()).expect("stage");
        age(&store);
        store.stage_import(8, staged()).expect("a stale stage gives way to a new import");
        age(&store);
        assert_eq!(store.take_import(8), None, "a commit after the operation's TTL finds no stage");
        store.stage_import(9, staged()).expect("stage");
        age(&store);
        store.evict_stale_import();
        assert!(store.import_stage.lock().is_none(), "the idle turn drops a stale stage");
    }

    #[test]
    fn a_refused_publish_frees_only_its_own_stage() {
        let store = RegistryStore::with_adapter_count(1);
        store.stage_import(7, staged()).expect("stage");
        store.discard_import(8);
        assert_eq!(store.stage_import(9, staged()), Err(ImportStageRefusal::Busy));
        store.discard_import(7);
        store.stage_import(9, staged()).expect("the stage is free once discarded");
    }
}
