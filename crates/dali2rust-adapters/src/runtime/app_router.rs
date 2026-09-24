use std::sync::Arc;

use dali2rust_api::confirmation_bridge::PendingConfirmationSlots;
use dali2rust_api::http::app::{AppBuilder, RouteKey};
use dali2rust_api::http::dispatcher::CorrelationIdAllocator;
use dali2rust_api::http::handler::ApiHandler;
use dali2rust_api::http::handlers::adapters::{
    AdapterGetHandler, AdapterPatchHandler, AdaptersListHandler,
};
use dali2rust_api::http::handlers::controller::ControllerSummaryHandler;
use dali2rust_api::http::handlers::groups::{
    GroupApplyHandler, GroupGetHandler, GroupMembershipMatrixGetHandler,
    GroupMembershipMatrixWriteHandler, GroupPatchHandler,
    GroupTargetStateHandler, GroupsListHandler,
};
use dali2rust_api::http::handlers::hcl::{
    HclBusContext, HclScheduleCreateHandler, HclScheduleDeleteHandler, HclScheduleDetailHandler,
    HclScheduleListHandler, HclSchedulePatchHandler,
};
use dali2rust_api::http::handlers::hcl_override::{HclOverrideDeleteHandler, HclOverrideGetHandler};
use dali2rust_api::http::handlers::json_command::{
    DaliCommandMapper, JsonCommandHandler, LevelMapper, RawMapper,
};
use dali2rust_api::http::handlers::operations::{OperationGetHandler, OperationsListHandler};
use dali2rust_api::http::handlers::commissioning::{
    CommissioningAddressChangeHandler, CommissioningIdentifyHandler, CommissioningReplacementHandler,
    CommissioningStepHandler,
};
use dali2rust_api::http::handlers::physical_devices::{
    AdapterDiscoveryRunsHandler, PhysicalDeviceAttributeReadsHandler,
    PhysicalDeviceAttributesHandler, PhysicalDeviceDeleteHandler, PhysicalDeviceGetHandler, PhysicalDeviceMemoryBanksHandler,
    PhysicalDevicePatchHandler, PhysicalDeviceTargetStateHandler,
    PhysicalDeviceWriteAttributesHandler, PhysicalDevicesListHandler,
};
use dali2rust_api::http::handlers::settings_home_assistant::{
    HomeAssistantDiscoveryPublishHandler, HomeAssistantSettingsGetHandler,
    HomeAssistantSettingsPatchHandler,
};
use dali2rust_api::http::handlers::config_transfer::{
    ConfigManifestHandler, ConfigSliceHandler,
};
use dali2rust_api::http::handlers::policies::{
    PoliciesApplyHandler, PoliciesGetHandler, PoliciesPatchHandler,
};
use dali2rust_api::http::handlers::redundancy::{
    RedundancyGetHandler, RedundancySwitchoverHandler,
};
use dali2rust_api::http::handlers::settings_dali::{
    DaliSettingsGetHandler, DaliSettingsPatchHandler,
};
use dali2rust_api::http::handlers::settings_redundancy::{
    RedundancySettingsGetHandler, RedundancySettingsPatchHandler,
};
use dali2rust_api::http::handlers::settings_poller::{
    PollerSettingsGetHandler, PollerSettingsPatchHandler,
};
use dali2rust_api::http::handlers::static_assets::{StaticAsset, StaticAssetHandler};
use dali2rust_api::http::handlers::scenes::{
    SceneApplyHandler, SceneGetHandler, SceneMatrixGetHandler, SceneMatrixWriteHandler,
    ScenePatchHandler, SceneRecallHandler, ScenesListHandler,
};
use dali2rust_api::http::handlers::virtual_lamps::{
    VirtualLampBindingDeleteHandler, VirtualLampBindingPutHandler, VirtualLampDeleteHandler,
    VirtualLampGetHandler,
    VirtualLampPatchHandler, VirtualLampTargetStateHandler, VirtualLampsListHandler,
};
use dali2rust_api::http::router::Router;
use dali2rust_api::http::stats_state::StatsHttpState;
use dali2rust_bus::{BusConfig, BusId, BusPublisher};
use dali2rust_domain::registry::{HclOverrideReadPort, OperationReadPort};
use dali2rust_operations_runtime::OperationTrackerHttpRead;
use dali2rust_platform::clock::Clock;

use super::http_bridges::RegistryHttpPorts;

pub(crate) struct ReadModelPorts {
    pub diagnostics: Arc<dyn dali2rust_api::http::diagnostics_state::DiagnosticsHttpState>,
    pub stats: Arc<dyn StatsHttpState>,
    pub redundancy: Arc<dyn dali2rust_api::http::redundancy_state::RedundancyHttpState>,
    pub firmware: Arc<dyn dali2rust_api::http::firmware_state::FirmwareHttpState>,
}

pub(crate) struct HttpBusDispatch {
    pub publisher: BusPublisher,
    pub slots: Arc<PendingConfirmationSlots>,
    pub correlation: Arc<CorrelationIdAllocator>,
    pub bus_id: BusId,
    pub confirmation_timeout_ms: u64,
}

impl HttpBusDispatch {
    pub(crate) fn for_bus(bus_config: BusConfig, bus_id: BusId, publisher: BusPublisher) -> Self {
        Self {
            publisher,
            slots: Arc::new(PendingConfirmationSlots::with_capacity(
                bus_config.confirmation_slots,
            )),
            correlation: Arc::new(CorrelationIdAllocator::new()),
            bus_id,
            confirmation_timeout_ms: bus_config.confirmation_timeout_ms,
        }
    }
}

type MutationCtor<S, W, H> = fn(
    BusPublisher,
    Arc<PendingConfirmationSlots>,
    Arc<CorrelationIdAllocator>,
    Arc<S>,
    Arc<W>,
    BusId,
    u64,
) -> H;

type TargetStateCtor<S, H> = fn(
    BusPublisher,
    Arc<PendingConfirmationSlots>,
    Arc<CorrelationIdAllocator>,
    Arc<S>,
    BusId,
    u64,
) -> H;

type ApplyCtor<S, H> =
    fn(BusPublisher, Arc<CorrelationIdAllocator>, Arc<S>, Arc<dyn OperationReadPort>, BusId) -> H;

fn mutation<S: ?Sized, W: ?Sized, H: ApiHandler + 'static>(
    bus: &HttpBusDispatch,
    state: &Arc<S>,
    watch: &Arc<W>,
    make: MutationCtor<S, W, H>,
) -> Box<dyn ApiHandler> {
    Box::new(make(
        bus.publisher.clone(),
        Arc::clone(&bus.slots),
        Arc::clone(&bus.correlation),
        Arc::clone(state),
        Arc::clone(watch),
        bus.bus_id,
        bus.confirmation_timeout_ms,
    ))
}

type ConfigWriteCtor<S, H> = fn(BusPublisher, Arc<CorrelationIdAllocator>, Arc<S>, BusId) -> H;

fn config_write<S: ?Sized, H: ApiHandler + 'static>(
    bus: &HttpBusDispatch,
    state: &Arc<S>,
    make: ConfigWriteCtor<S, H>,
) -> Box<dyn ApiHandler> {
    Box::new(make(
        bus.publisher.clone(),
        Arc::clone(&bus.correlation),
        Arc::clone(state),
        bus.bus_id,
    ))
}

type MutationWithWallCtor<S, W, H> = fn(
    BusPublisher,
    Arc<PendingConfirmationSlots>,
    Arc<CorrelationIdAllocator>,
    Arc<S>,
    Arc<W>,
    Arc<dyn dali2rust_platform::clock::UnixTimeMs>,
    BusId,
    u64,
) -> H;

fn mutation_with_wall<S: ?Sized, W: ?Sized, H: ApiHandler + 'static>(
    bus: &HttpBusDispatch,
    state: &Arc<S>,
    watch: &Arc<W>,
    wall: &Arc<dyn dali2rust_platform::clock::UnixTimeMs>,
    make: MutationWithWallCtor<S, W, H>,
) -> Box<dyn ApiHandler> {
    Box::new(make(
        bus.publisher.clone(),
        Arc::clone(&bus.slots),
        Arc::clone(&bus.correlation),
        Arc::clone(state),
        Arc::clone(watch),
        Arc::clone(wall),
        bus.bus_id,
        bus.confirmation_timeout_ms,
    ))
}

fn target_state<S: ?Sized, H: ApiHandler + 'static>(
    bus: &HttpBusDispatch,
    state: &Arc<S>,
    make: TargetStateCtor<S, H>,
) -> Box<dyn ApiHandler> {
    Box::new(make(
        bus.publisher.clone(),
        Arc::clone(&bus.slots),
        Arc::clone(&bus.correlation),
        Arc::clone(state),
        bus.bus_id,
        bus.confirmation_timeout_ms,
    ))
}

fn apply<S: ?Sized, H: ApiHandler + 'static>(
    bus: &HttpBusDispatch,
    state: &Arc<S>,
    op_read: &Arc<dyn OperationReadPort>,
    make: ApplyCtor<S, H>,
) -> Box<dyn ApiHandler> {
    Box::new(make(
        bus.publisher.clone(),
        Arc::clone(&bus.correlation),
        Arc::clone(state),
        Arc::clone(op_read),
        bus.bus_id,
    ))
}

fn read<S: ?Sized, H: ApiHandler + 'static>(
    state: &Arc<S>,
    make: fn(Arc<S>) -> H,
) -> Box<dyn ApiHandler> {
    Box::new(make(Arc::clone(state)))
}

pub(crate) fn build_app_router(
    version: &'static str,
    adapter_count: u8,
    bus: HttpBusDispatch,
    registry: RegistryHttpPorts,
    op_read: Arc<dyn OperationReadPort>,
    hcl_overrides: Arc<dyn HclOverrideReadPort>,
    read_models: ReadModelPorts,
    clock: Arc<dyn Clock>,
    wall_clock: Arc<dyn dali2rust_platform::wall_clock::WallClock>,
    persist_timezone: Arc<dyn Fn(&str) + Send + Sync>,
    web_assets: &'static [StaticAsset],
    home_assistant: Arc<dyn dali2rust_api::http::handlers::controller::ControllerHaSummary>,
    rules: RulesHttpDeps,
) -> Router {
    let redundancy_read = Arc::clone(&read_models.redundancy);
    let base = AppBuilder::new(version, Arc::clone(&clock))
        .with_role_port(Arc::clone(&registry.role_port));
    let builder = wire_read_models(base, read_models, &bus);
    let builder = wire_controller_and_adapters(
        builder,
        version,
        adapter_count,
        &bus,
        &registry,
        clock,
        home_assistant,
    );
    let builder = wire_dali_command_handlers(builder, &bus);
    let builder = wire_operations(builder, &op_read);
    let builder = wire_physical_devices(builder, adapter_count, &bus, &registry);
    let builder = wire_commissioning(builder, adapter_count, &bus, &registry, &op_read);
    let builder = wire_groups(builder, &bus, &registry, &op_read);
    let builder = wire_scenes(builder, &bus, &registry, &op_read);
    let builder = wire_hcl(builder, &bus, &registry, &hcl_overrides);
    let builder = wire_input_devices(builder, &bus, &registry);
    let builder = wire_rules(builder, &bus, &rules);
    let builder = wire_settings_and_lamps(builder, &bus, &registry, &redundancy_read);
    let builder = builder
        .with_handler(
            RouteKey::Time,
            Box::new(dali2rust_api::http::handlers::TimeHandler::new(
                Arc::clone(&wall_clock),
                Arc::clone(&persist_timezone),
            )),
        )
        .with_handler(
            RouteKey::TimeSet,
            Box::new(dali2rust_api::http::handlers::TimeHandler::new(
                wall_clock,
                persist_timezone,
            )),
        );
    let builder = wire_web_ui(builder, web_assets);
    builder.build()
}

#[inline(never)]
fn wire_read_models(
    builder: AppBuilder,
    read_models: ReadModelPorts,
    bus: &HttpBusDispatch,
) -> AppBuilder {
    let builder = wire_firmware(builder, bus, &read_models.firmware);
    builder
        .with_handler(
            RouteKey::Diagnostics,
            Box::new(
                dali2rust_api::http::handlers::diagnostics::DiagnosticsHandler::new(
                    read_models.diagnostics,
                ),
            ),
        )
        .with_handler(
            RouteKey::Stats,
            Box::new(dali2rust_api::http::handlers::stats::StatsHandler::new(
                read_models.stats,
            )),
        )
}

#[inline(never)]
fn wire_controller_and_adapters(
    builder: AppBuilder,
    version: &'static str,
    adapter_count: u8,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
    clock: Arc<dyn Clock>,
    home_assistant: Arc<dyn dali2rust_api::http::handlers::controller::ControllerHaSummary>,
) -> AppBuilder {
    builder
        .with_handler(
            RouteKey::Controller,
            Box::new(ControllerSummaryHandler::new(
                version,
                adapter_count,
                clock,
                home_assistant,
            )),
        )
        .with_handler(RouteKey::AdaptersList, read(&registry.adapter_state, AdaptersListHandler::new))
        .with_handler(RouteKey::AdapterGet, read(&registry.adapter_state, AdapterGetHandler::new))
        .with_handler(RouteKey::AdapterPatch, Box::new(AdapterPatchHandler::new(
            bus.publisher.clone(),
            Arc::clone(&bus.slots),
            Arc::clone(&bus.correlation),
            Arc::clone(&registry.adapter_state),
            Arc::clone(&registry.adapter_apply_watch),
            bus.confirmation_timeout_ms,
        )))
}

#[inline(never)]
fn wire_dali_command_handlers(builder: AppBuilder, bus: &HttpBusDispatch) -> AppBuilder {
    let args = (
        bus.publisher.clone(),
        Arc::clone(&bus.slots),
        Arc::clone(&bus.correlation),
        bus.bus_id,
        bus.confirmation_timeout_ms,
    );
    builder
        .with_handler(RouteKey::DaliCommand, Box::new(JsonCommandHandler::<DaliCommandMapper>::new(
            args.0.clone(),
            args.1.clone(),
            args.2.clone(),
            args.3,
            args.4,
        )))
        .with_handler(RouteKey::DaliLevel, Box::new(JsonCommandHandler::<LevelMapper>::new(
            args.0.clone(),
            args.1.clone(),
            args.2.clone(),
            args.3,
            args.4,
        )))
        .with_handler(RouteKey::DaliRaw, Box::new(JsonCommandHandler::<RawMapper>::new(
            args.0, args.1, args.2, args.3, args.4,
        )))
}

#[inline(never)]
fn wire_operations(builder: AppBuilder, op_read: &Arc<dyn OperationReadPort>) -> AppBuilder {
    builder
        .with_handler(RouteKey::OperationGet, read(op_read, OperationGetHandler::new))
        .with_handler(RouteKey::OperationsList, read(op_read, OperationsListHandler::new))
}

fn wire_physical_device_reads(
    builder: AppBuilder,
    registry: &RegistryHttpPorts,
    wall: &Arc<dyn dali2rust_platform::clock::UnixTimeMs>,
) -> AppBuilder {
    builder
        .with_handler(
            RouteKey::PhysicalDevicesList,
            Box::new(PhysicalDevicesListHandler::new(
                Arc::clone(&registry.physical_state),
                Arc::clone(wall),
            )),
        )
        .with_handler(
            RouteKey::PhysicalDeviceGet,
            Box::new(PhysicalDeviceGetHandler::new(
                Arc::clone(&registry.physical_state),
                Arc::clone(wall),
            )),
        )
        .with_handler(
            RouteKey::PhysicalDeviceAttributes,
            Box::new(PhysicalDeviceAttributesHandler::new(
                Arc::clone(&registry.physical_state),
                Arc::clone(wall),
            )),
        )
        .with_handler(
            RouteKey::PhysicalDeviceMemoryBanks,
            Box::new(PhysicalDeviceMemoryBanksHandler::new(
                Arc::clone(&registry.physical_state),
                Arc::clone(wall),
            )),
        )
}

#[inline(never)]
fn wire_physical_devices(
    builder: AppBuilder,
    adapter_count: u8,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
) -> AppBuilder {
    let wall: Arc<dyn dali2rust_platform::clock::UnixTimeMs> =
        Arc::new(dali2rust_bsp::unix_clock::StdUnixTimeMs);
    let builder = wire_physical_device_reads(builder, registry, &wall)
        .with_handler(
            RouteKey::PhysicalDevicePatch,
            mutation_with_wall(bus, &registry.physical_state, &registry.pd_apply_watch, &wall, PhysicalDevicePatchHandler::new),
        )
        .with_handler(
            RouteKey::PhysicalDeviceDelete,
            mutation(bus, &registry.physical_state, &registry.pd_apply_watch, PhysicalDeviceDeleteHandler::new),
        )
        .with_handler(
            RouteKey::PhysicalDeviceTargetState,
            target_state(bus, &registry.physical_state, PhysicalDeviceTargetStateHandler::new),
        );
    wire_physical_device_diagnostics(builder, adapter_count, bus, registry)
}

#[inline(never)]
fn wire_commissioning(
    builder: AppBuilder,
    adapter_count: u8,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
    op_read: &Arc<dyn OperationReadPort>,
) -> AppBuilder {
    let builder = wire_commissioning_operations(builder, bus, registry, op_read);
    wire_commissioning_expert(builder, adapter_count, bus, op_read)
}

#[inline(never)]
fn wire_commissioning_operations(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
    op_read: &Arc<dyn OperationReadPort>,
) -> AppBuilder {
    builder
        .with_handler(
            RouteKey::CommissioningIdentify,
            Box::new(CommissioningIdentifyHandler::new(
                bus.publisher.clone(),
                Arc::clone(&bus.correlation),
                bus.bus_id,
                Arc::clone(&registry.physical_state),
                Arc::clone(op_read),
            )),
        )
        .with_handler(
            RouteKey::CommissioningAddressChanges,
            Box::new(CommissioningAddressChangeHandler::new(
                bus.publisher.clone(),
                Arc::clone(&bus.correlation),
                bus.bus_id,
                Arc::clone(&registry.physical_state),
                Arc::clone(op_read),
            )),
        )
        .with_handler(
            RouteKey::CommissioningReplacements,
            Box::new(CommissioningReplacementHandler::new(
                bus.publisher.clone(),
                Arc::clone(&bus.correlation),
                bus.bus_id,
                Arc::clone(&registry.physical_state),
                Arc::clone(op_read),
            )),
        )
}

#[inline(never)]
fn wire_commissioning_expert(
    builder: AppBuilder,
    adapter_count: u8,
    bus: &HttpBusDispatch,
    op_read: &Arc<dyn OperationReadPort>,
) -> AppBuilder {
    builder.with_handler(
        RouteKey::CommissioningSteps,
        Box::new(CommissioningStepHandler::new(
            bus.publisher.clone(),
            Arc::clone(&bus.slots),
            Arc::clone(&bus.correlation),
            bus.bus_id,
            adapter_count,
            Arc::clone(op_read),
            bus.confirmation_timeout_ms,
        )),
    )
}

#[inline(never)]
fn wire_physical_device_diagnostics(
    builder: AppBuilder,
    adapter_count: u8,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
) -> AppBuilder {
    builder
        .with_handler(RouteKey::PhysicalDeviceWriteAttrs, Box::new(
            PhysicalDeviceWriteAttributesHandler::new(
                bus.publisher.clone(),
                Arc::clone(&bus.correlation),
                bus.bus_id,
                adapter_count,
            ),
        ))
        .with_handler(RouteKey::AdapterDiscoveryRuns, Box::new(AdapterDiscoveryRunsHandler::new(
            bus.publisher.clone(),
            Arc::clone(&bus.correlation),
            bus.bus_id,
            adapter_count,
        )))
        .with_handler(RouteKey::PhysicalDeviceAttributeReads, Box::new(
            PhysicalDeviceAttributeReadsHandler::new(
                bus.publisher.clone(),
                Arc::clone(&bus.correlation),
                bus.bus_id,
                Arc::clone(&registry.physical_state),
            ),
        ))
}

#[inline(never)]
fn wire_virtual_lamps(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
) -> AppBuilder {
    builder
        .with_handler(RouteKey::VirtualLampsList, read(&registry.vl_state, VirtualLampsListHandler::new))
        .with_handler(RouteKey::VirtualLampGet, read(&registry.vl_state, VirtualLampGetHandler::new))
        .with_handler(
            RouteKey::VirtualLampPatch,
            mutation(bus, &registry.vl_state, &registry.vl_patch_watch, VirtualLampPatchHandler::new),
        )
        .with_handler(
            RouteKey::VirtualLampBindingPut,
            mutation(bus, &registry.vl_state, &registry.vl_binding_watch, VirtualLampBindingPutHandler::new),
        )
        .with_handler(
            RouteKey::VirtualLampBindingDelete,
            mutation(bus, &registry.vl_state, &registry.vl_binding_watch, VirtualLampBindingDeleteHandler::new),
        )
        .with_handler(
            RouteKey::VirtualLampDelete,
            mutation(bus, &registry.vl_state, &registry.vl_patch_watch, VirtualLampDeleteHandler::new),
        )
        .with_handler(
            RouteKey::VirtualLampTargetState,
            target_state(bus, &registry.vl_state, VirtualLampTargetStateHandler::new),
        )
}

#[inline(never)]
fn wire_groups(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
    op_read: &Arc<dyn OperationReadPort>,
) -> AppBuilder {
    let state = &registry.group_state;
    builder
        .with_handler(RouteKey::GroupsList, read(state, GroupsListHandler::new))
        .with_handler(RouteKey::GroupGet, read(state, GroupGetHandler::new))
        .with_handler(RouteKey::GroupPatch, mutation(bus, state, &registry.group_metadata_watch, GroupPatchHandler::new))
        .with_handler(RouteKey::GroupMembershipMatrixGet, read(state, GroupMembershipMatrixGetHandler::new))
        .with_handler(RouteKey::GroupMembershipMatrixPatch, config_write(bus, state, GroupMembershipMatrixWriteHandler::patch))
        .with_handler(RouteKey::GroupMembershipMatrixPut, config_write(bus, state, GroupMembershipMatrixWriteHandler::put))
        .with_handler(RouteKey::GroupApply, apply(bus, state, op_read, GroupApplyHandler::new))
        .with_handler(RouteKey::GroupTargetState, target_state(bus, state, GroupTargetStateHandler::new))
}

#[inline(never)]
fn wire_scenes(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
    op_read: &Arc<dyn OperationReadPort>,
) -> AppBuilder {
    let state = &registry.scene_state;
    builder
        .with_handler(RouteKey::ScenesList, read(state, ScenesListHandler::new))
        .with_handler(RouteKey::SceneGet, read(state, SceneGetHandler::new))
        .with_handler(RouteKey::ScenePatch, mutation(bus, state, &registry.scene_metadata_watch, ScenePatchHandler::new))
        .with_handler(RouteKey::SceneMatrixGet, read(state, SceneMatrixGetHandler::new))
        .with_handler(RouteKey::SceneMatrixPatch, config_write(bus, state, SceneMatrixWriteHandler::patch))
        .with_handler(RouteKey::SceneMatrixPut, config_write(bus, state, SceneMatrixWriteHandler::put))
        .with_handler(RouteKey::SceneApply, apply(bus, state, op_read, SceneApplyHandler::new))
        .with_handler(RouteKey::SceneRecall, target_state(bus, state, SceneRecallHandler::new))
}

#[inline(never)]
fn wire_hcl(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
    overrides: &Arc<dyn HclOverrideReadPort>,
) -> AppBuilder {
    let state = &registry.hcl_state;
    let ctx = HclBusContext {
        publisher: bus.publisher.clone(),
        slots: Arc::clone(&bus.slots),
        correlation: Arc::clone(&bus.correlation),
        bus_id: bus.bus_id,
        timeout_ms: bus.confirmation_timeout_ms,
    };
    builder
        .with_handler(RouteKey::HclSchedulesList, read(state, HclScheduleListHandler::new))
        .with_handler(RouteKey::HclScheduleGet, read(state, HclScheduleDetailHandler::new))
        .with_handler(
            RouteKey::HclScheduleCreate,
            Box::new(HclScheduleCreateHandler::new(Arc::clone(state), ctx.clone())),
        )
        .with_handler(
            RouteKey::HclSchedulePatch,
            Box::new(HclSchedulePatchHandler::new(Arc::clone(state), ctx.clone())),
        )
        .with_handler(
            RouteKey::HclScheduleDelete,
            Box::new(HclScheduleDeleteHandler::new(Arc::clone(state), ctx.clone())),
        )
        .with_handler(
            RouteKey::HclOverrideGet,
            Box::new(HclOverrideGetHandler::new(
                Arc::clone(state),
                Arc::clone(overrides),
            )),
        )
        .with_handler(
            RouteKey::HclOverrideDelete,
            Box::new(HclOverrideDeleteHandler::new(Arc::clone(state), ctx)),
        )
}

#[inline(never)]
fn wire_input_devices(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
) -> AppBuilder {
    use dali2rust_api::http::handlers::input_devices::{
        InputDeviceGetHandler, InputDeviceListHandler,
    };
    let state = &registry.input_device_state;
    let wall: Arc<dyn dali2rust_platform::clock::UnixTimeMs> =
        Arc::new(dali2rust_bsp::unix_clock::StdUnixTimeMs);
    let builder = builder
        .with_handler(
            RouteKey::InputDevicesList,
            Box::new(InputDeviceListHandler::new(Arc::clone(state))),
        )
        .with_handler(
            RouteKey::InputDeviceGet,
            Box::new(InputDeviceGetHandler::new(Arc::clone(state), Arc::clone(&wall))),
        );
    wire_input_device_actions(builder, bus, registry)
}

#[inline(never)]
fn wire_input_device_actions(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
) -> AppBuilder {
    use dali2rust_api::http::handlers::input_devices::{
        InputDeviceAction, InputDeviceActionHandler,
    };
    let state = &registry.input_device_state;
    let action = |kind: InputDeviceAction| -> Box<InputDeviceActionHandler> {
        Box::new(InputDeviceActionHandler::new(
            bus.publisher.clone(),
            bus.bus_id,
            Arc::clone(&bus.correlation),
            Arc::clone(state),
            kind,
        ))
    };
    builder
        .with_handler(RouteKey::InputDevicesScan, action(InputDeviceAction::Scan))
        .with_handler(
            RouteKey::InputDevicesCommission,
            action(InputDeviceAction::Commission),
        )
        .with_handler(
            RouteKey::InputDeviceIdentify,
            action(InputDeviceAction::Identify),
        )
        .with_handler(
            RouteKey::InputDeviceInstancePatch,
            action(InputDeviceAction::ConfigureInstance),
        )
        .with_handler(
            RouteKey::InputDeviceFeedbackPatch,
            action(InputDeviceAction::ConfigureFeedback),
        )
        .with_handler(
            RouteKey::InputDevicePatch,
            action(InputDeviceAction::PatchMetadata),
        )
        .with_handler(RouteKey::InputDeviceDelete, action(InputDeviceAction::Forget))
}

pub(crate) struct RulesHttpDeps {
    pub state: Arc<dyn dali2rust_api::http::rules_state::RulesHttpState>,
    pub compiler: Arc<dyn dali2rust_rules_model::RuleCompiler>,
    pub resolver: Arc<dyn dali2rust_rules_model::NameResolver>,
}

#[inline(never)]
fn wire_rules(builder: AppBuilder, bus: &HttpBusDispatch, deps: &RulesHttpDeps) -> AppBuilder {
    use dali2rust_api::http::handlers::rules::{RulesAction, RulesHandler, RulesHandlerShared};
    let shared = Arc::new(RulesHandlerShared::new(
        Arc::clone(&deps.state),
        Arc::clone(&deps.compiler),
        Arc::clone(&deps.resolver),
        bus.publisher.clone(),
        bus.bus_id,
        Arc::clone(&bus.correlation),
    ));
    let handler = |action: RulesAction| -> Box<RulesHandler> {
        Box::new(RulesHandler::new(Arc::clone(&shared), action))
    };
    builder
        .with_handler(RouteKey::RulesGet, handler(RulesAction::Get))
        .with_handler(RouteKey::RulesParse, handler(RulesAction::Parse))
        .with_handler(RouteKey::RulesPut, handler(RulesAction::Put))
        .with_handler(RouteKey::RulesRulePatch, handler(RulesAction::PatchRule))
        .with_handler(RouteKey::RulesRuleRun, handler(RulesAction::RunRule))
}

#[inline(never)]
fn wire_firmware(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    firmware: &Arc<dyn dali2rust_api::http::firmware_state::FirmwareHttpState>,
) -> AppBuilder {
    use dali2rust_api::http::handlers::firmware::{FirmwareGetHandler, FirmwareUpdateHandler};
    builder
        .with_handler(RouteKey::Firmware, read(firmware, FirmwareGetHandler::new))
        .with_handler(
            RouteKey::FirmwareUpdates,
            Box::new(FirmwareUpdateHandler::new(
                bus.publisher.clone(),
                Arc::clone(&bus.correlation),
                Arc::clone(firmware),
                bus.bus_id,
            )),
        )
}

#[inline(never)]
fn wire_settings_poller(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
) -> AppBuilder {
    let state = &registry.poller_state;
    builder
        .with_handler(RouteKey::SettingsPollerGet, read(state, PollerSettingsGetHandler::new))
        .with_handler(
            RouteKey::SettingsPollerPatch,
            mutation(bus, state, &registry.poller_apply_watch, PollerSettingsPatchHandler::new),
        )
}

#[inline(never)]
fn wire_settings_dali(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
) -> AppBuilder {
    let state = &registry.dali_settings_state;
    builder
        .with_handler(RouteKey::SettingsDaliGet, read(state, DaliSettingsGetHandler::new))
        .with_handler(
            RouteKey::SettingsDaliPatch,
            mutation(bus, state, &registry.dali_settings_apply_watch, DaliSettingsPatchHandler::new),
        )
}

#[inline(never)]
fn wire_settings_and_lamps(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
    redundancy_read: &Arc<dyn dali2rust_api::http::redundancy_state::RedundancyHttpState>,
) -> AppBuilder {
    let builder = wire_settings_poller(builder, bus, registry);
    let builder = wire_settings_dali(builder, bus, registry);
    let builder = wire_redundancy(builder, bus, registry, redundancy_read);
    let builder = wire_policies(builder, bus, registry);
    let builder = wire_config_transfer(builder, bus, &registry.config_transfer);
    let builder = wire_settings_home_assistant(builder, bus, registry);
    wire_virtual_lamps(builder, bus, registry)
}

#[inline(never)]
fn wire_config_transfer(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    transfer: &Arc<dyn dali2rust_api::http::handlers::config_transfer::ConfigTransferPort>,
) -> AppBuilder {
    let slice = || {
        Box::new(ConfigSliceHandler::new(
            Arc::clone(transfer),
            bus.publisher.clone(),
            Arc::clone(&bus.slots),
            Arc::clone(&bus.correlation),
            bus.bus_id,
            bus.confirmation_timeout_ms,
        ))
    };
    builder
        .with_handler(
            RouteKey::ConfigManifest,
            Box::new(ConfigManifestHandler::new(Arc::clone(transfer))),
        )
        .with_handler(RouteKey::ConfigSlice, slice())
        .with_handler(RouteKey::ConfigSlicePut, slice())
}

#[inline(never)]
fn wire_policies(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
) -> AppBuilder {
    let state = &registry.policies_state;
    builder
        .with_handler(RouteKey::PoliciesGet, read(state, PoliciesGetHandler::new))
        .with_handler(
            RouteKey::PoliciesPatch,
            mutation(
                bus,
                state,
                &registry.policies_apply_watch,
                PoliciesPatchHandler::new,
            ),
        )
        .with_handler(
            RouteKey::PoliciesApply,
            Box::new(PoliciesApplyHandler::new(
                bus.publisher.clone(),
                Arc::clone(&bus.correlation),
                Arc::clone(state),
                bus.bus_id,
                REDUNDANCY_REGISTRY_ADAPTER_ID,
            )),
        )
}

#[inline(never)]
fn wire_redundancy(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
    redundancy_state: &Arc<dyn dali2rust_api::http::redundancy_state::RedundancyHttpState>,
) -> AppBuilder {
    let state = &registry.redundancy_settings_state;
    builder
        .with_handler(
            RouteKey::SettingsRedundancyGet,
            read(state, RedundancySettingsGetHandler::new),
        )
        .with_handler(
            RouteKey::SettingsRedundancyPatch,
            mutation(
                bus,
                state,
                &registry.redundancy_settings_apply_watch,
                RedundancySettingsPatchHandler::new,
            ),
        )
        .with_handler(
            RouteKey::Redundancy,
            read(redundancy_state, RedundancyGetHandler::new),
        )
        .with_handler(
            RouteKey::RedundancySwitchover,
            Box::new(RedundancySwitchoverHandler::new(
                bus.publisher.clone(),
                Arc::clone(&bus.slots),
                Arc::clone(&bus.correlation),
                Arc::clone(redundancy_state),
                Arc::clone(&registry.redundancy_settings_state),
                bus.bus_id,
                REDUNDANCY_REGISTRY_ADAPTER_ID,
                bus.confirmation_timeout_ms,
            )),
        )
}

const REDUNDANCY_REGISTRY_ADAPTER_ID: u8 = 0;

#[inline(never)]
fn wire_settings_home_assistant(
    builder: AppBuilder,
    bus: &HttpBusDispatch,
    registry: &RegistryHttpPorts,
) -> AppBuilder {
    let state = &registry.home_assistant_state;
    builder
        .with_handler(
            RouteKey::SettingsHomeAssistantGet,
            read(state, HomeAssistantSettingsGetHandler::new),
        )
        .with_handler(
            RouteKey::SettingsHomeAssistantPatch,
            mutation(
                bus,
                state,
                &registry.home_assistant_apply_watch,
                HomeAssistantSettingsPatchHandler::new,
            ),
        )
        .with_handler(
            RouteKey::SettingsHomeAssistantDiscoveryPublish,
            Box::new(HomeAssistantDiscoveryPublishHandler::new(
                bus.publisher.clone(),
                Arc::clone(&bus.correlation),
                bus.bus_id,
            )),
        )
}

#[inline(never)]
fn wire_web_ui(builder: AppBuilder, web_assets: &'static [StaticAsset]) -> AppBuilder {
    if web_assets.is_empty() {
        return builder;
    }
    builder
        .with_handler(RouteKey::WebUiRoot, Box::new(StaticAssetHandler::new(web_assets)))
        .with_handler(RouteKey::WebUiFallback, Box::new(StaticAssetHandler::new(web_assets)))
}

pub(crate) fn operation_read_port(
    tracker: Arc<std::sync::Mutex<dali2rust_operations_runtime::OperationTrackerInner>>,
) -> Arc<dyn OperationReadPort> {
    Arc::new(OperationTrackerHttpRead(tracker))
}

pub(crate) fn hcl_override_read_port(
    ledger: dali2rust_hcl_runtime::SharedOverrideLedger,
) -> Arc<dyn HclOverrideReadPort> {
    Arc::new(dali2rust_hcl_runtime::HclOverrideLedgerRead::new(ledger))
}
