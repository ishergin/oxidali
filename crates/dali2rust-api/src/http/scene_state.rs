use std::sync::Arc;

use dali2rust_domain::registry::{
    SceneApplySnapshot, SceneMatrixRowView, SceneMatrixView,
    SceneMetadataWatchPort, SceneReadPort, SceneRowStateView, SceneView,
};
use serde::Serialize;

use super::adapter_state::AdapterHttpState;
use super::physical_device_state::{
    capabilities_view_to_dto, CapabilityFlagsDto, RgbDto, WafDto, XyDto,
};

#[derive(Clone, Debug, Serialize)]
pub struct SceneDto {
    pub adapter_id: u8,
    pub scene_id: u8,
    pub name: String,
    pub ha_select_enabled: bool,
    pub row_count_included: u8,
    pub dirty: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SceneRowStateDto {
    pub included: bool,
    pub power: Option<String>,
    pub level: Option<u8>,
    pub color_mode: Option<String>,
    pub color_temperature_kelvin: Option<u16>,
    pub xy: Option<XyDto>,
    pub rgb: Option<RgbDto>,
    pub waf: Option<WafDto>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SceneMatrixRowDto {
    pub virtual_lamp_id: u8,
    pub name: String,
    pub capabilities: CapabilityFlagsDto,
    pub desired: SceneRowStateDto,
    pub applied: SceneRowStateDto,
    pub dirty: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SceneMatrixDto {
    pub adapter_id: u8,
    pub scene_id: u8,
    pub rows: Vec<SceneMatrixRowDto>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScenesListBody {
    pub adapter_id: u8,
    pub scenes: Vec<SceneDto>,
}

pub fn scene_view_to_dto(view: SceneView) -> SceneDto {
    SceneDto {
        adapter_id: view.adapter_id,
        scene_id: view.scene_id,
        name: view.name,
        ha_select_enabled: view.ha_select_enabled,
        row_count_included: view.row_count_included,
        dirty: view.dirty,
    }
}

fn row_state_to_dto(view: SceneRowStateView) -> SceneRowStateDto {
    SceneRowStateDto {
        included: view.included,
        power: view.power,
        level: view.level,
        color_mode: view.color_mode,
        color_temperature_kelvin: view.color_temperature_kelvin,
        xy: view.xy.map(|(x, y)| XyDto::from_wire(x, y)),
        rgb: view.rgb.map(|(r, g, b)| RgbDto { r, g, b }),
        waf: view.waf.map(|(w, a, f)| WafDto { w, a, f }),
    }
}

fn matrix_row_to_dto(row: SceneMatrixRowView) -> SceneMatrixRowDto {
    SceneMatrixRowDto {
        virtual_lamp_id: row.virtual_lamp_id,
        name: row.name,
        capabilities: capabilities_view_to_dto(&row.capabilities),
        desired: row_state_to_dto(row.desired),
        applied: row_state_to_dto(row.applied),
        dirty: row.dirty,
    }
}

pub fn scene_matrix_view_to_dto(view: SceneMatrixView) -> SceneMatrixDto {
    SceneMatrixDto {
        adapter_id: view.adapter_id,
        scene_id: view.scene_id,
        rows: view.rows.into_iter().map(matrix_row_to_dto).collect(),
    }
}

pub trait SceneMetadataApplyWatch: Send + Sync {
    fn scene_metadata_applied_load(&self) -> u32;
}

pub struct SceneMetadataApplyWatchBridge {
    port: Arc<dyn SceneMetadataWatchPort>,
}

impl SceneMetadataApplyWatchBridge {
    pub fn new(port: Arc<dyn SceneMetadataWatchPort>) -> Self {
        Self { port }
    }
}

impl SceneMetadataApplyWatch for SceneMetadataApplyWatchBridge {
    fn scene_metadata_applied_load(&self) -> u32 {
        self.port.scene_metadata_applied_load()
    }
}

pub trait SceneHttpState: AdapterHttpState {
    fn scene_dto(&self, adapter_id: u8, scene_id: u8) -> Option<SceneDto>;
    fn list_scene_dtos(&self, adapter_id: u8) -> Vec<SceneDto>;
    fn scene_matrix_dto(&self, adapter_id: u8, scene_id: u8) -> Option<SceneMatrixDto>;
    fn scene_apply_snapshot(&self, adapter_id: u8, scene_id: u8) -> Option<SceneApplySnapshot>;
}

pub struct SceneHttpStateBridge {
    port: Arc<dyn SceneReadPort>,
}

impl SceneHttpStateBridge {
    pub fn new(port: Arc<dyn SceneReadPort>) -> Self {
        Self { port }
    }
}

crate::http::adapter_state::impl_adapter_http_state_from_port!(SceneHttpStateBridge);

impl SceneHttpState for SceneHttpStateBridge {
    fn scene_dto(&self, adapter_id: u8, scene_id: u8) -> Option<SceneDto> {
        self.port
            .scene_view(adapter_id, scene_id)
            .map(scene_view_to_dto)
    }

    fn list_scene_dtos(&self, adapter_id: u8) -> Vec<SceneDto> {
        self.port
            .list_scene_views(adapter_id)
            .into_iter()
            .map(scene_view_to_dto)
            .collect()
    }

    fn scene_matrix_dto(&self, adapter_id: u8, scene_id: u8) -> Option<SceneMatrixDto> {
        self.port
            .scene_matrix_view(adapter_id, scene_id)
            .map(scene_matrix_view_to_dto)
    }

    fn scene_apply_snapshot(&self, adapter_id: u8, scene_id: u8) -> Option<SceneApplySnapshot> {
        self.port.scene_apply_snapshot(adapter_id, scene_id)
    }
}
