use dali2rust_domain::registry::VIRTUAL_LAMP_COUNT;
use dali2rust_contracts::msg::{
    ConfigWriteResource, GroupMatrixDesiredRow, SceneMatrixDesiredRow,
};

use super::store::{evict_stale, registry_unix_ms, Inner, RegistryStore, Staged, STAGE_MAX_AGE_MS};

pub(crate) const MAX_CONFIG_WRITE_STAGES: usize = 2;

pub(crate) const CONFIG_WRITE_STAGE_MAX_AGE_MS: u64 = STAGE_MAX_AGE_MS;

const MAX_STAGED_ROWS: usize = VIRTUAL_LAMP_COUNT as usize;

pub(crate) type ConfigWriteStageKey = (ConfigWriteResource, u8, u8);

pub(crate) enum StagedRows {
    Group(Vec<GroupMatrixDesiredRow>),
    Scene(Vec<SceneMatrixDesiredRow>),
}

pub(crate) struct ConfigWriteStage {
    pub(crate) rows: StagedRows,
    pub(crate) chunks: u8,
    pub(crate) staged_at_ms: u64,
    pub(crate) series: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigWriteRejection {
    TooManyStages,
    TooManyRows,
    NoStage,
    ChunkCountMismatch,
}

impl ConfigWriteRejection {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::TooManyStages => "config_write_stage_limit",
            Self::TooManyRows => "config_write_too_many_rows",
            Self::NoStage => "config_write_no_stage",
            Self::ChunkCountMismatch => "config_write_chunk_count_mismatch",
        }
    }
}

impl RegistryStore {
    pub(crate) fn stage_scene_matrix_chunk(
        &self,
        series: u64,
        adapter_id: u8,
        scene_id: u8,
        rows: &[SceneMatrixDesiredRow],
    ) -> Result<(), ConfigWriteRejection> {
        let key = (ConfigWriteResource::SceneMatrix, adapter_id, scene_id);
        let mut inner = self.write_inner();
        let outcome = stage_rows(&mut inner, key, series, |staged| match staged {
            StagedRows::Scene(dst) => {
                dst.extend_from_slice(rows);
                dst.len()
            }
            StagedRows::Group(dst) => dst.len(),
        });
        evict_on_error(&mut inner, key, outcome)
    }

    pub(crate) fn stage_group_matrix_chunk(
        &self,
        series: u64,
        adapter_id: u8,
        rows: &[GroupMatrixDesiredRow],
    ) -> Result<(), ConfigWriteRejection> {
        let key = (ConfigWriteResource::GroupMatrix, adapter_id, 0);
        let mut inner = self.write_inner();
        let outcome = stage_rows(&mut inner, key, series, |staged| match staged {
            StagedRows::Group(dst) => {
                dst.extend_from_slice(rows);
                dst.len()
            }
            StagedRows::Scene(dst) => dst.len(),
        });
        evict_on_error(&mut inner, key, outcome)
    }

    pub(crate) fn commit_config_write(
        &self,
        series: u64,
        resource: ConfigWriteResource,
        adapter_id: u8,
        scene_id: u8,
        chunks: u8,
    ) -> Result<bool, ConfigWriteRejection> {
        let key = (resource, adapter_id, scene_id);
        let staged = {
            let mut inner = self.write_inner();
            let current = inner
                .config_write_stage
                .get(&key)
                .ok_or(ConfigWriteRejection::NoStage)?;
            if current.series != series {
                return Err(ConfigWriteRejection::NoStage);
            }
            if current.chunks != chunks {
                inner.config_write_stage.remove(&key);
                return Err(ConfigWriteRejection::ChunkCountMismatch);
            }
            inner
                .config_write_stage
                .remove(&key)
                .ok_or(ConfigWriteRejection::NoStage)?
        };
        Ok(match staged.rows {
            StagedRows::Scene(rows) => self.apply_scene_matrix_rows(adapter_id, scene_id, &rows),
            StagedRows::Group(rows) => self.apply_group_matrix_rows(adapter_id, &rows),
        })
    }

    pub(crate) fn evict_stale_config_write_stages(&self, max_age_ms: u64) {
        let now = registry_unix_ms();
        evict_stale(&mut self.write_inner().config_write_stage, now, max_age_ms);
    }
}

fn stage_rows(
    inner: &mut Inner,
    key: ConfigWriteStageKey,
    series: u64,
    append: impl FnOnce(&mut StagedRows) -> usize,
) -> Result<(), ConfigWriteRejection> {
    if inner
        .config_write_stage
        .get(&key)
        .is_some_and(|stage| stage.series != series)
    {
        inner.config_write_stage.remove(&key);
    }
    if !inner.config_write_stage.contains_key(&key)
        && inner.config_write_stage.len() >= MAX_CONFIG_WRITE_STAGES
    {
        return Err(ConfigWriteRejection::TooManyStages);
    }
    let staged_at_ms = registry_unix_ms();
    let stage = inner
        .config_write_stage
        .entry(key)
        .or_insert_with(|| ConfigWriteStage {
            rows: match key.0 {
                ConfigWriteResource::GroupMatrix => StagedRows::Group(Vec::new()),
                ConfigWriteResource::SceneMatrix => StagedRows::Scene(Vec::new()),
            },
            chunks: 0,
            staged_at_ms,
            series,
        });
    let len = append(&mut stage.rows);
    stage.chunks = stage.chunks.saturating_add(1);
    if len > MAX_STAGED_ROWS {
        return Err(ConfigWriteRejection::TooManyRows);
    }
    Ok(())
}

fn evict_on_error(
    inner: &mut Inner,
    key: ConfigWriteStageKey,
    outcome: Result<(), ConfigWriteRejection>,
) -> Result<(), ConfigWriteRejection> {
    if outcome.is_err() {
        inner.config_write_stage.remove(&key);
    }
    outcome
}

impl Staged for ConfigWriteStage {
    fn staged_at_ms(&self) -> u64 {
        self.staged_at_ms
    }
}
