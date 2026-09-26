use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use crate::runtime::registry::conversions::{
    capabilities_view, color_mode_str, device_type_str, override_source_str,
};
use crate::runtime::registry::runtime_fields::RuntimeObservationData;
use crate::runtime::registry::store::RegistryStore;
use crate::runtime::registry::views::{
    apply_dt8_capability_flags, effective_capabilities_view, physical_device_state_view,
};
use dali2rust_bsp::psram::PsramBox;
use dali2rust_contracts::msg::{
    observation_supersedes, ColorMode, ColorValue, DeviceType, DeviceTypeSet, ExtendedVersionEntry,
    FailureStatus, MAX_EXTENDED_VERSIONS,
    FixedText64, LastDapcSource, LevelTransition, LightSetpoint, PowerState, RuntimeObservation,
    RuntimeSource, StatusFlags,
};
use dali2rust_domain::dali::level_transition::{LevelContext, TransitionOutcome};
use dali2rust_domain::registry::{
    MemoryBusUnitAttributesView, MemoryDiagnosticsAttributesView, MemoryEnergyAttributesView,
    MemoryLuminaireAttributesView,
    AttributeSectionKind, AttributeSectionView,
    CapabilityFlagsView, ColorTemperatureRangeView, MemoryBankRangeView, MemoryBankSummaryView, ObservedValue,
    PhysicalDeviceAttributesView, PhysicalDeviceCoreView, PhysicalDeviceReadPort,
    PhysicalDeviceSummaryView, PhysicalDeviceView,
};

const DISCOVERY_EVICT_MISS_THRESHOLD: u8 = 3;

const SCENE_COLOUR_TYPE_MASK: u8 = 0xFF;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MemoryBankRangeRecord {
    pub start: u16,
    pub length: u16,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MemoryBankRecord {
    pub bank: u8,
    pub total_bytes_read: u16,
    pub last_read_ms: u64,
    pub ranges: Vec<MemoryBankRangeRecord>,
    pub bytes: Vec<u8>,
}

pub(crate) struct PhysicalRuntimeCommit<'a> {
    pub short_address: u8,
    pub setpoint: &'a LightSetpoint,
    pub observation: &'a RuntimeObservation,
    pub observed_at_mono_ms: Option<u32>,
    pub entry_last_dapc_source: Option<LastDapcSource>,
    pub entry_source: RuntimeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeCommitOutcome {
    Committed,
    Unchanged,
    Superseded,
    NoRecord,
}

fn setpoint_states_a_value(setpoint: &LightSetpoint) -> bool {
    setpoint.power != PowerState::Unknown || setpoint.level > 0 || setpoint.color.is_some()
}

pub(crate) struct LevelTransitionCommit<'a> {
    pub short_address: u8,
    pub transition: LevelTransition,
    pub observation: &'a RuntimeObservation,
    pub source: RuntimeSource,
    pub observed_at_mono_ms: Option<u32>,
}

fn resolve_transition(
    transition: LevelTransition,
    rec: &PhysicalDeviceRecord,
) -> TransitionOutcome {
    dali2rust_domain::dali::level_transition::resolve(
        transition,
        &LevelContext {
            target_level: rec.runtime_level,
            min_level: rec.attributes.common102.min_level.as_ref().map(|v| v.value),
            max_level: rec.attributes.common102.max_level.as_ref().map(|v| v.value),
            last_active_level: rec.last_active_level,
        },
    )
}

// IEC 62386-102 Table 14
// IEC 62386-209 Table 8
fn forget_ram_state(rec: &mut PhysicalDeviceRecord) {
    rec.last_active_level = rec.attributes.common102.max_level.as_ref().map(|v| v.value);
    let dt8 = &mut rec.attributes.dt8_color;
    dt8.color_value_0 = None;
    dt8.color_value_1 = None;
    dt8.color_value_2 = None;
    dt8.gear_features = None;
    dt8.rgbwaf_control = None;
    rec.runtime.forget_colour();
}

fn commit_physical_runtime(
    rec: &mut PhysicalDeviceRecord,
    commit: &PhysicalRuntimeCommit<'_>,
) -> bool {
    let power_cycled = rec.runtime.power_cycle_began(commit.observation);
    if power_cycled {
        forget_ram_state(rec);
    }
    if commit.setpoint.power == PowerState::Off || commit.setpoint.level > 0 {
        rec.runtime_level = Some(commit.setpoint.level);
    } else if commit.setpoint.power == PowerState::On {
        rec.runtime_level = rec.last_active_level;
    }
    // IEC 62386-102 §9.4
    if let Some(level) = rec.runtime_level.filter(|level| *level > 0 && !power_cycled) {
        rec.last_active_level = Some(level);
    }
    rec.runtime.apply_setpoint(commit.setpoint);
    rec.runtime.apply_observation(commit.observation);
    rec.runtime.merge_command_metadata(
        commit.observation.value_source,
        commit.entry_last_dapc_source,
        commit.entry_source,
        setpoint_states_a_value(commit.setpoint),
    );
    rec.runtime.observed_at_mono_ms = commit
        .observed_at_mono_ms
        .or(rec.runtime.observed_at_mono_ms);
    power_cycled
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SceneColourObservation {
    pub colour_type: u8,
    pub values: [Option<u16>; 6],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SceneColourEvidence {
    #[default]
    Unread,
    NoColour,
    Colour(SceneColourObservation),
}

#[derive(Clone, Debug)]
pub(crate) struct PhysicalDeviceRecord {
    pub name: FixedText64,
    pub notes: Option<FixedText64>,
    pub random_address: Option<u32>,
    pub device_type_discovered: DeviceType,
    pub device_type_override: Option<DeviceType>,
    pub supported_device_types: Option<DeviceTypeSet>,
    pub color_mode_discovered: ColorMode,
    pub color_mode_override: Option<ColorMode>,
    pub dt8_auto_activation_repair: Option<bool>,
    pub dt8_rgbwaf_control_assert: Option<bool>,
    pub attributes: PhysicalDeviceAttributesView,
    pub runtime_level: Option<u8>,
    pub last_active_level: Option<u8>,
    pub runtime: RuntimeObservationData,
    pub tc_coolest_mirek: Option<u16>,
    pub tc_warmest_mirek: Option<u16>,
    pub cap_brightness: bool,
    pub cap_cct: bool,
    pub cap_xy: bool,
    pub cap_rgb: bool,
    pub cap_rgbwaf: bool,
    pub memory_banks: Vec<MemoryBankRecord>,
    pub energy: MemoryEnergyAttributesView,
    pub diagnostics: MemoryDiagnosticsAttributesView,
    pub bus_unit: MemoryBusUnitAttributesView,
    pub luminaire_info: Option<PsramBox<MemoryLuminaireAttributesView>>,
    pub scene_colour_observed: [SceneColourEvidence; 16],
    pub extended_versions: [Option<ExtendedVersionEntry>; MAX_EXTENDED_VERSIONS],
    pub scan_miss_count: u8,
}

macro_rules! physical_device_record_fields {
    (
        placed { $( $placed:ident : $placed_ty:ty ),* $(,)? }
        inline { $( $inline:ident : $inline_ty:ty = $default:expr ),* $(,)? }
    ) => {
        impl PhysicalDeviceRecord {
            /// # Safety
            /// `p` must be a live, aligned, unaliased, uninitialised `Self`.
            unsafe fn write_fields(p: *mut Self) {
                // SAFETY: the caller's contract; each field is written exactly once and none is read.
                unsafe {
                    $(
                        let field = addr_of_mut!((*p).$placed).cast::<MaybeUninit<$placed_ty>>();
                        <$placed_ty>::default_in_place(&mut *field);
                    )*
                    $(
                        addr_of_mut!((*p).$inline).write($default);
                    )*
                }
            }

            #[cfg(test)]
            pub(crate) fn empty(short_address: u8) -> Self {
                let _ = short_address;
                Self {
                    $( $placed: <$placed_ty>::default(), )*
                    $( $inline: $default, )*
                }
            }

            #[cfg(test)]
            const INLINE_INIT_TEMPORARIES: &'static [(&'static str, usize)] = &[
                $( (stringify!($inline), core::mem::size_of::<$inline_ty>()), )*
            ];
        }
    };
}

physical_device_record_fields! {
    placed {
        attributes: PhysicalDeviceAttributesView,
        diagnostics: MemoryDiagnosticsAttributesView,
    }
    inline {
        name: FixedText64 = FixedText64::new(),
        notes: Option<FixedText64> = None,
        random_address: Option<u32> = None,
        device_type_discovered: DeviceType = DeviceType::Unknown,
        device_type_override: Option<DeviceType> = None,
        supported_device_types: Option<DeviceTypeSet> = None,
        color_mode_discovered: ColorMode = ColorMode::Unknown,
        color_mode_override: Option<ColorMode> = None,
        dt8_auto_activation_repair: Option<bool> = None,
        dt8_rgbwaf_control_assert: Option<bool> = None,
        runtime_level: Option<u8> = None,
        last_active_level: Option<u8> = None,
        runtime: RuntimeObservationData = RuntimeObservationData::default(),
        tc_coolest_mirek: Option<u16> = None,
        tc_warmest_mirek: Option<u16> = None,
        cap_brightness: bool = true,
        cap_cct: bool = false,
        cap_xy: bool = false,
        cap_rgb: bool = false,
        cap_rgbwaf: bool = false,
        memory_banks: Vec<MemoryBankRecord> = Vec::new(),
        energy: MemoryEnergyAttributesView = MemoryEnergyAttributesView::default(),
        bus_unit: MemoryBusUnitAttributesView = MemoryBusUnitAttributesView::default(),
        luminaire_info: Option<PsramBox<MemoryLuminaireAttributesView>> = None,
        scene_colour_observed: [SceneColourEvidence; 16] = [SceneColourEvidence::Unread; 16],
        extended_versions: [Option<ExtendedVersionEntry>; MAX_EXTENDED_VERSIONS] =
            [None; MAX_EXTENDED_VERSIONS],
        scan_miss_count: u8 = 0,
    }
}

impl PhysicalDeviceRecord {
    pub(crate) fn new_boxed() -> PsramBox<Self> {
        // SAFETY: `init_empty` writes every field of the place exactly once.
        unsafe { PsramBox::new_with(Self::init_empty) }
    }

    pub(crate) fn init_empty(place: &mut MaybeUninit<Self>) {
        // SAFETY: `place` is live, aligned and unaliased; `write_fields` initialises every field.
        unsafe { Self::write_fields(place.as_mut_ptr()) }
    }

    pub(crate) fn effective_device_type(&self) -> DeviceType {
        self.device_type_override
            .unwrap_or(self.device_type_discovered)
    }

    pub(crate) fn effective_color_mode(&self) -> ColorMode {
        self.color_mode_override
            .unwrap_or(self.color_mode_discovered)
    }

    pub(crate) fn capability_flags(&self) -> CapabilityFlagsView {
        capabilities_view(
            self.cap_brightness,
            self.cap_cct,
            self.cap_xy,
            self.cap_rgb,
            self.cap_rgbwaf,
        )
    }

    pub(crate) fn set_capability_flags(&mut self, caps: CapabilityFlagsView) {
        self.cap_brightness = caps.brightness;
        self.cap_cct = caps.cct;
        self.cap_xy = caps.xy;
        self.cap_rgb = caps.rgb;
        self.cap_rgbwaf = caps.rgbwaf;
    }
}

fn merge_attr<T: PartialEq>(
    slot: &mut Option<ObservedValue<T>>,
    value: T,
    source: dali2rust_domain::registry::AttributeSource,
    now_ms: u64,
) -> bool {
    use dali2rust_domain::registry::AttributeSource;
    let changed = slot
        .as_ref()
        .is_none_or(|prev| prev.value != value || prev.source != source);
    let (last_read_ms, last_write_confirmed_ms) = match source {
        AttributeSource::Readback => (
            Some(now_ms),
            slot.as_ref().and_then(|prev| prev.last_write_confirmed_ms),
        ),
        AttributeSource::WriteConfirmed => (
            slot.as_ref().and_then(|prev| prev.last_read_ms),
            Some(now_ms),
        ),
    };
    *slot = Some(ObservedValue {
        value,
        source,
        last_read_ms,
        last_write_confirmed_ms,
    });
    changed
}

pub(crate) fn merge_read_attr<T: PartialEq>(
    slot: &mut Option<ObservedValue<T>>,
    value: T,
    now_ms: u64,
) -> bool {
    merge_attr(
        slot,
        value,
        dali2rust_domain::registry::AttributeSource::Readback,
        now_ms,
    )
}

pub(crate) fn merge_read_attr_opt<T: PartialEq>(
    slot: &mut Option<ObservedValue<T>>,
    value: Option<T>,
    now_ms: u64,
) -> bool {
    value.is_some_and(|v| merge_read_attr(slot, v, now_ms))
}

fn merge_write_attr<T: Copy + PartialEq>(
    slot: &mut Option<ObservedValue<T>>,
    value: T,
    now_ms: u64,
) -> bool {
    merge_attr(
        slot,
        value,
        dali2rust_domain::registry::AttributeSource::WriteConfirmed,
        now_ms,
    )
}

fn merge_write_attr_opt<T: Copy + PartialEq>(
    slot: &mut Option<ObservedValue<T>>,
    value: Option<T>,
    now_ms: u64,
) -> bool {
    value.is_some_and(|v| merge_write_attr(slot, v, now_ms))
}

fn replace_changed<T: PartialEq>(slot: &mut T, next: T) -> bool {
    if *slot == next {
        false
    } else {
        *slot = next;
        true
    }
}

fn merge_groups_membership_read(
    attrs: &mut PhysicalDeviceAttributesView,
    membership: u16,
    now_ms: u64,
) -> bool {
    merge_read_attr(&mut attrs.groups.membership, membership, now_ms)
}

pub(crate) fn write_groups_membership_read(
    attrs: &mut PhysicalDeviceAttributesView,
    membership: u16,
    now_ms: u64,
) -> bool {
    merge_groups_membership_read(attrs, membership, now_ms)
}

pub(crate) fn write_scene_level_read(
    attrs: &mut PhysicalDeviceAttributesView,
    scene_id: u8,
    level: u8,
    now_ms: u64,
) -> bool {
    attrs
        .scenes
        .levels
        .get_mut(scene_id as usize)
        .is_some_and(|slot| merge_read_attr(slot, level, now_ms))
}

pub(crate) fn write_scene_level_written(
    attrs: &mut PhysicalDeviceAttributesView,
    scene_id: u8,
    level: u8,
    now_ms: u64,
) -> bool {
    attrs
        .scenes
        .levels
        .get_mut(scene_id as usize)
        .is_some_and(|slot| merge_write_attr(slot, level, now_ms))
}

pub(crate) fn forget_scene_level(attrs: &mut PhysicalDeviceAttributesView, scene_id: u8) -> bool {
    attrs
        .scenes
        .levels
        .get_mut(scene_id as usize)
        .is_some_and(|slot| slot.take().is_some())
}

fn merge_scene_levels_read(
    attrs: &mut PhysicalDeviceAttributesView,
    levels: &[u8],
    now_ms: u64,
) -> bool {
    let mut changed = false;
    for (scene, level) in levels.iter().copied().enumerate() {
        changed |= merge_read_attr(&mut attrs.scenes.levels[scene], level, now_ms);
    }
    changed
}

fn merge_dt6_read_snapshot(
    attrs: &mut PhysicalDeviceAttributesView,
    dt6: &dali2rust_contracts::msg::Dt6ReadSnapshot,
    now_ms: u64,
) -> bool {
    let led = &mut attrs.dt6_led;
    let mut changed = merge_read_attr_opt(&mut led.gear_type, dt6.gear_type, now_ms);
    changed |= merge_read_attr_opt(&mut led.dimming_curve, dt6.dimming_curve, now_ms);
    changed |= merge_read_attr_opt(
        &mut led.possible_operating_mode,
        dt6.possible_operating_mode,
        now_ms,
    );
    changed |= merge_read_attr_opt(&mut led.features, dt6.features, now_ms);
    changed |= merge_dt6_failure_reads(attrs, dt6, now_ms);
    let led = &mut attrs.dt6_led;
    changed |= merge_read_attr_opt(
        &mut led.current_protector_enabled,
        dt6.current_protector_enabled,
        now_ms,
    );
    changed |= merge_read_attr_opt(&mut led.operating_mode, dt6.operating_mode, now_ms);
    changed |= merge_read_attr_opt(&mut led.fast_fade_time, dt6.fast_fade_time, now_ms);
    changed |= merge_read_attr_opt(&mut led.min_fast_fade_time, dt6.min_fast_fade_time, now_ms);
    changed |= merge_read_attr_opt(
        &mut led.extended_version_number,
        dt6.extended_version_number,
        now_ms,
    );
    changed
}

fn merge_dt6_failure_reads(
    attrs: &mut PhysicalDeviceAttributesView,
    dt6: &dali2rust_contracts::msg::Dt6ReadSnapshot,
    now_ms: u64,
) -> bool {
    let led = &mut attrs.dt6_led;
    let mut changed = merge_read_attr_opt(&mut led.failure_status, dt6.failure_status, now_ms);
    changed |= merge_read_attr_opt(&mut led.short_circuit, dt6.short_circuit, now_ms);
    changed |= merge_read_attr_opt(&mut led.open_circuit, dt6.open_circuit, now_ms);
    changed |= merge_read_attr_opt(&mut led.load_decrease, dt6.load_decrease, now_ms);
    changed |= merge_read_attr_opt(&mut led.load_increase, dt6.load_increase, now_ms);
    changed |= merge_read_attr_opt(
        &mut led.current_protector_active,
        dt6.current_protector_active,
        now_ms,
    );
    changed |= merge_read_attr_opt(&mut led.thermal_shutdown, dt6.thermal_shutdown, now_ms);
    changed |= merge_read_attr_opt(&mut led.thermal_overload, dt6.thermal_overload, now_ms);
    changed |= merge_read_attr_opt(&mut led.reference_running, dt6.reference_running, now_ms);
    changed |= merge_read_attr_opt(
        &mut led.reference_measurement_failed,
        dt6.reference_measurement_failed,
        now_ms,
    );
    changed
}

fn luminaire_view(rec: &PhysicalDeviceRecord) -> Option<Box<MemoryLuminaireAttributesView>> {
    rec.luminaire_info.as_deref().map(|v| Box::new(v.clone()))
}

fn memory_banks_view(banks: &[MemoryBankRecord]) -> Vec<MemoryBankSummaryView> {
    banks
        .iter()
        .map(|bank| MemoryBankSummaryView {
            bank: bank.bank,
            total_bytes_read: bank.total_bytes_read,
            last_read_ms: bank.last_read_ms,
            ranges: bank
                .ranges
                .iter()
                .map(|range| MemoryBankRangeView {
                    start: range.start,
                    length: range.length,
                })
                .collect(),
        })
        .collect()
}

fn status_flags_data_to_msg(
    sf: &crate::runtime::registry::runtime_fields::StatusFlagsData,
) -> StatusFlags {
    StatusFlags {
        raw: sf.raw,
        lamp_failure: sf.lamp_failure,
        gear_failure: sf.gear_failure,
        lamp_on: sf.lamp_on,
        limit_error: sf.limit_error,
        fade_running: sf.fade_running,
        reset_state: sf.reset_state,
        missing_short_address: sf.missing_short_address,
        power_cycle_seen: sf.power_cycle_seen,
    }
}

fn failure_status_data_to_msg(
    fs: &crate::runtime::registry::runtime_fields::FailureStatusData,
) -> FailureStatus {
    FailureStatus {
        raw: fs.raw,
        lamp_failure: fs.lamp_failure,
        gear_failure: fs.gear_failure,
        communication_failure: fs.communication_failure,
        source: fs.source,
    }
}

fn color_value_from_runtime(rec: &RuntimeObservationData) -> Option<ColorValue> {
    let base = |mode| ColorValue {
        mode,
        ..ColorValue::default()
    };
    match rec.color_mode {
        ColorMode::Cct => rec.kelvin.map(|k| ColorValue {
            color_temperature_kelvin: k,
            ..base(ColorMode::Cct)
        }),
        ColorMode::Xy => rec.xy.map(|(x, y)| ColorValue {
            x: (x.clamp(0.0, 1.0) * 65535.0) as u16,
            y: (y.clamp(0.0, 1.0) * 65535.0) as u16,
            ..base(ColorMode::Xy)
        }),
        ColorMode::Rgb => rec.rgb.map(|(r, g, b)| ColorValue {
            r,
            g,
            b,
            ..base(ColorMode::Rgb)
        }),
        ColorMode::Rgbwaf => match (rec.rgb, rec.waf) {
            (Some((r, g, b)), Some((w, a, f))) => Some(ColorValue {
                r,
                g,
                b,
                w,
                a,
                f,
                ..base(ColorMode::Rgbwaf)
            }),
            _ => None,
        },
        _ => None,
    }
}

fn light_setpoint_from_physical_record(rec: &PhysicalDeviceRecord) -> LightSetpoint {
    LightSetpoint {
        power: rec.runtime.power,
        level: rec.runtime_level.unwrap_or(0),
        color: color_value_from_runtime(&rec.runtime),
    }
}

fn runtime_observation_from_runtime_data(rt: &RuntimeObservationData) -> RuntimeObservation {
    RuntimeObservation {
        status_flags: rt.status_flags.as_ref().map(status_flags_data_to_msg),
        failure_status: rt.failure_status.as_ref().map(failure_status_data_to_msg),
        value_source: rt.value_source,
        last_seen_ms: rt.last_seen_ms,
        last_dapc_source: rt.last_dapc_source,
        error: rt.error.clone(),
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ForgetOutcome {
    pub(crate) unbound_lamps: Vec<u8>,
    pub(crate) groups_changed: bool,
}

fn lamps_bound_to(inner: &super::store::Inner, adapter_id: u8, short_address: u8) -> Vec<u8> {
    let mut ids: Vec<u8> = inner
        .lamps
        .iter()
        .filter(|((aid, _), entry)| {
            *aid == adapter_id && entry.binding_short == Some(short_address)
        })
        .map(|((_, lamp_id), _)| *lamp_id)
        .collect();
    ids.sort_unstable();
    ids
}

impl RegistryStore {

    fn merge_declared_device_types(
        r: &mut PhysicalDeviceRecord,
        observed: Option<DeviceTypeSet>,
    ) -> bool {
        let Some(observed) = observed else {
            return false;
        };
        let merged = match r.supported_device_types {
            Some(stored) => DeviceTypeSet::from_bits(stored.bits() | observed.bits()),
            None => observed,
        };
        replace_changed(&mut r.supported_device_types, Some(merged))
    }

    #[cfg(test)]
    pub(crate) fn seed_discovered_dt8(&self, short_address: u8, random_address: Option<u32>) -> bool {
        self.apply_discovery_progress(&dali2rust_contracts::msg::DaliDiscoveryProgressEvent {
            registry_adapter_id: 0,
            short_address,
            random_address,
            device_type: DeviceType::Dt8Color,
            color_mode: ColorMode::Cct,
            dt8_xy_capable: false,
            dt8_tc_capable: true,
            dt8_rgb_capable: false,
            dt8_rgbwaf_capable: false,
            supported_device_types: None,
        })
    }

    pub(crate) fn apply_discovery_progress(
        &self,
        progress: &dali2rust_contracts::msg::DaliDiscoveryProgressEvent,
    ) -> bool {
        let adapter_id = progress.registry_adapter_id;
        let short_address = progress.short_address;
        if usize::from(short_address) >= super::store::MAX_SHORT_ADDRESSES {
            log::warn!(
                "registry: rejected discovery progress with out-of-range short {short_address}"
            );
            return false;
        }
        let mut g = self.write_inner();
        let r = g
            .physical_devices
            .entry((adapter_id, short_address))
            .or_insert_with(PhysicalDeviceRecord::new_boxed);
        if progress.random_address.is_some() {
            r.random_address = progress.random_address;
        }
        r.scan_miss_count = 0;
        r.device_type_discovered = progress.device_type;
        r.color_mode_discovered = progress.color_mode;
        if progress.supported_device_types.is_some() {
            r.supported_device_types = progress.supported_device_types;
        }
        apply_dt8_capability_flags(
            r,
            progress.dt8_tc_capable,
            progress.dt8_xy_capable,
            progress.dt8_rgb_capable,
            progress.dt8_rgbwaf_capable,
            progress.color_mode,
        );
        g.physical_devices_revision = g.physical_devices_revision.saturating_add(1);
        drop(g);
        self.dirty.mark_physical_device_dirty(adapter_id, short_address);
        true
    }

    pub(crate) fn reconcile_discovery_scan(
        &self,
        adapter_id: u8,
        confirmed_mask: u64,
    ) -> Vec<u8> {
        let mut evicted: Vec<u8> = Vec::new();
        let mut g = self.write_inner();
        for ((aid, sa), r) in g.physical_devices.iter_mut() {
            if *aid != adapter_id {
                continue;
            }
            if confirmed_mask & (1u64 << (sa & 0x3F)) != 0 {
                r.scan_miss_count = 0;
                continue;
            }
            r.scan_miss_count = r.scan_miss_count.saturating_add(1);
            if r.scan_miss_count >= DISCOVERY_EVICT_MISS_THRESHOLD && r.random_address.is_none() {
                evicted.push(*sa);
            }
        }
        for sa in &evicted {
            g.physical_devices.remove(&(adapter_id, *sa));
            log::warn!(
                "registry: evicted never-verified physical device a{adapter_id} short {sa} \
                 after {DISCOVERY_EVICT_MISS_THRESHOLD} clean-scan misses"
            );
        }
        if !evicted.is_empty() {
            g.physical_devices_revision = g.physical_devices_revision.saturating_add(1);
            drop(g);
            for sa in &evicted {
                self.dirty.mark_physical_device_dirty(adapter_id, *sa);
            }
        }
        evicted
    }

    pub(crate) fn apply_physical_device_forget(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<ForgetOutcome> {
        let mut g = self.write_inner();
        g.physical_devices.remove(&(adapter_id, short_address))?;
        g.physical_devices_revision = g.physical_devices_revision.saturating_add(1);
        let unbound = lamps_bound_to(&g, adapter_id, short_address);
        let mut groups_changed = false;
        for lamp_id in &unbound {
            if let Some(entry) = g.lamps.get_mut(&(adapter_id, *lamp_id)) {
                entry.binding_short = None;
            }
            groups_changed |= super::groups::forget_adopted_desired_on_binding_change(
                &mut g,
                adapter_id,
                *lamp_id,
            );
        }
        drop(g);
        self.dirty.mark_physical_device_dirty(adapter_id, short_address);
        if !unbound.is_empty() {
            self.dirty.mark_virtual_lamps_dirty(adapter_id);
        }
        if groups_changed {
            self.dirty.mark_groups_dirty(adapter_id);
        }
        Some(ForgetOutcome { unbound_lamps: unbound, groups_changed })
    }

    pub(crate) fn apply_physical_device_attribute_chunk(
        &self,
        adapter_id: u8,
        short_address: u8,
        chunk: &dali2rust_contracts::msg::DaliAttributeReadChunk,
    ) -> bool {
        let mut g = self.write_inner();
        let Some(r) = g.physical_devices.get_mut(&(adapter_id, short_address)) else {
            return false;
        };
        let now = crate::runtime::registry::store::registry_unix_ms();
        let changed = Self::apply_attribute_chunk(r, chunk, now);
        g.physical_devices_revision = g.physical_devices_revision.saturating_add(1);
        drop(g);
        if changed {
            self.dirty.mark_physical_device_dirty(adapter_id, short_address);
        }
        true
    }

    fn apply_attribute_chunk(
        r: &mut PhysicalDeviceRecord,
        chunk: &dali2rust_contracts::msg::DaliAttributeReadChunk,
        now: u64,
    ) -> bool {
        use dali2rust_contracts::msg::DaliAttributeReadChunk as Chunk;
        match chunk {
            Chunk::Identity { random_address } => {
                replace_changed(&mut r.random_address, Some(*random_address))
            }
            Chunk::Common102 { .. } => Self::apply_attributes_common_102(r, chunk, now),
            Chunk::Dt8Color { .. } => Self::apply_attributes_dt8(r, chunk, now),
            Chunk::Groups { membership } => match membership {
                Some(v) => merge_groups_membership_read(&mut r.attributes, *v, now),
                None => replace_changed(&mut r.attributes.groups.membership, None),
            },
            Chunk::Scenes { levels } => levels
                .as_ref()
                .is_some_and(|levels| merge_scene_levels_read(&mut r.attributes, levels, now)),
            Chunk::Dt6Led { snapshot } => merge_dt6_read_snapshot(&mut r.attributes, snapshot, now),
            Chunk::Extended {
                fade_time_ms,
                version_number,
            } => {
                merge_read_attr_opt(&mut r.attributes.extended.fade_time_ms, *fade_time_ms, now)
                    | merge_read_attr_opt(
                        &mut r.attributes.extended.version_number,
                        *version_number,
                        now,
                    )
            }
            Chunk::RuntimeStatus { .. } => false,
            Chunk::SceneColour {
                scene,
                level,
                colour_type,
                values,
            } => Self::apply_scene_colour_chunk(r, *scene, *level, *colour_type, values, now),
            Chunk::ExtendedVersions { versions } => {
                r.extended_versions = *versions;
                false
            }
        }
    }

    fn apply_scene_colour_chunk(
        r: &mut PhysicalDeviceRecord,
        scene: u8,
        level: Option<u8>,
        colour_type: Option<u8>,
        values: &[Option<u16>; 6],
        now: u64,
    ) -> bool {
        let mut changed = false;
        if let Some(level) = level {
            changed |= write_scene_level_read(&mut r.attributes, scene, level, now);
        }
        if let Some(colour_type) = colour_type {
            let evidence = if colour_type == SCENE_COLOUR_TYPE_MASK {
                SceneColourEvidence::NoColour
            } else {
                SceneColourEvidence::Colour(SceneColourObservation {
                    colour_type,
                    values: *values,
                })
            };
            if let Some(slot) = r.scene_colour_observed.get_mut(usize::from(scene)) {
                changed |= replace_changed(slot, evidence);
            }
        }
        changed
    }

    fn apply_attributes_common_102(
        r: &mut PhysicalDeviceRecord,
        chunk: &dali2rust_contracts::msg::DaliAttributeReadChunk,
        now: u64,
    ) -> bool {
        let dali2rust_contracts::msg::DaliAttributeReadChunk::Common102 {
            version,
            device_type,
            physical_minimum,
            min_level,
            max_level,
            power_on_level,
            system_failure_level,
            fade_time_ms,
            fade_rate,
            supported_device_types,
            light_source_type,
            light_source_types,
        } = chunk
        else {
            return false;
        };
        let mut changed = replace_changed(&mut r.cap_brightness, true);
        changed |= Self::merge_declared_device_types(r, *supported_device_types);
        let c102 = &mut r.attributes.common102;
        for (slot, value) in [
            (&mut c102.version, version),
            (&mut c102.device_type, device_type),
            (&mut c102.physical_minimum, physical_minimum),
            (&mut c102.min_level, min_level),
            (&mut c102.max_level, max_level),
            (&mut c102.power_on_level, power_on_level),
            (&mut c102.system_failure_level, system_failure_level),
            (&mut c102.light_source_type, light_source_type),
        ] {
            changed |= merge_read_attr_opt(slot, *value, now);
        }
        changed |= merge_read_attr_opt(&mut c102.fade_time_ms, *fade_time_ms, now);
        changed |= merge_read_attr_opt(&mut c102.fade_rate, *fade_rate, now);
        changed |= merge_read_attr_opt(&mut c102.light_source_types, *light_source_types, now);
        changed
    }

    fn apply_attributes_dt8(
        r: &mut PhysicalDeviceRecord,
        chunk: &dali2rust_contracts::msg::DaliAttributeReadChunk,
        now: u64,
    ) -> bool {
        let dali2rust_contracts::msg::DaliAttributeReadChunk::Dt8Color {
            color_mode,
            xy_capable,
            tc_capable,
            rgb_capable,
            color_type,
            color_value_0,
            color_value_1,
            color_value_2,
            tc_coolest_mirek,
            tc_warmest_mirek,
            gear_features,
            rgbwaf_capable,
            rgbwaf_control,
        } = chunk
        else {
            return false;
        };
        let mut changed = Self::apply_dt8_discovery_evidence(
            r,
            *color_mode,
            *xy_capable,
            *tc_capable,
            *rgb_capable,
            *rgbwaf_capable,
            (*tc_coolest_mirek, *tc_warmest_mirek),
        );
        let dt8 = &mut r.attributes.dt8_color;
        changed |= merge_read_attr_opt(&mut dt8.color_type, *color_type, now);
        changed |= merge_read_attr_opt(&mut dt8.color_value_0, *color_value_0, now);
        changed |= merge_read_attr_opt(&mut dt8.color_value_1, *color_value_1, now);
        changed |= merge_read_attr_opt(&mut dt8.color_value_2, *color_value_2, now);
        changed |= merge_read_attr_opt(&mut dt8.gear_features, *gear_features, now);
        changed |= merge_read_attr_opt(&mut dt8.rgbwaf_control, *rgbwaf_control, now);
        changed
    }

    fn apply_dt8_discovery_evidence(
        r: &mut PhysicalDeviceRecord,
        color_mode: ColorMode,
        xy_capable: bool,
        tc_capable: bool,
        rgb_capable: bool,
        rgbwaf_capable: bool,
        tc_range_mirek: (Option<u16>, Option<u16>),
    ) -> bool {
        let mut changed = replace_changed(&mut r.device_type_discovered, DeviceType::Dt8Color);
        if let (Some(coolest), Some(warmest)) = tc_range_mirek {
            changed |= replace_changed(&mut r.tc_coolest_mirek, Some(coolest));
            changed |= replace_changed(&mut r.tc_warmest_mirek, Some(warmest));
        }
        if color_mode != ColorMode::Unknown {
            changed |= replace_changed(&mut r.color_mode_discovered, color_mode);
        }
        changed |= apply_dt8_capability_flags(
            r,
            tc_capable,
            xy_capable,
            rgb_capable,
            rgbwaf_capable,
            color_mode,
        );
        changed
    }

    pub(crate) fn apply_physical_device_override(
        &self,
        patch: &dali2rust_contracts::msg::PhysicalDeviceOverrideCommand,
    ) -> bool {
        use dali2rust_contracts::msg::PhysicalDeviceOverrideCommand as Patch;
        let mut g = self.write_inner();
        let key = (patch.adapter_id, patch.short_address);
        let Some(rec) = g.physical_devices.get_mut(&key) else {
            return false;
        };
        if patch.patch_mask & Patch::PATCH_NAME != 0 {
            rec.name = dali2rust_contracts::msg::fixed_text_64(patch.name.as_str());
        }
        if patch.patch_mask & Patch::PATCH_DEVICE_TYPE_OVERRIDE != 0 {
            rec.device_type_override =
                (!patch.clear_device_type_override).then_some(patch.device_type_override);
        }
        if patch.patch_mask & Patch::PATCH_COLOR_MODE_OVERRIDE != 0 {
            rec.color_mode_override =
                (!patch.clear_color_mode_override).then_some(patch.color_mode_override);
        }
        if patch.patch_mask & Patch::PATCH_DT8_AUTO_ACTIVATION_REPAIR != 0 {
            rec.dt8_auto_activation_repair = Some(patch.dt8_auto_activation_repair);
        }
        if patch.patch_mask & Patch::PATCH_DT8_RGBWAF_CONTROL_ASSERT != 0 {
            rec.dt8_rgbwaf_control_assert = Some(patch.dt8_rgbwaf_control_assert);
        }
        drop(g);
        self.dirty
            .mark_physical_device_dirty(patch.adapter_id, patch.short_address);
        true
    }

    pub(crate) fn apply_physical_device_notes(
        &self,
        adapter_id: u8,
        short_address: u8,
        notes: &str,
    ) -> bool {
        let mut g = self.write_inner();
        let Some(rec) = g.physical_devices.get_mut(&(adapter_id, short_address)) else {
            return false;
        };
        rec.notes = (!notes.is_empty()).then(|| dali2rust_contracts::msg::fixed_text_64(notes));
        drop(g);
        self.dirty.mark_physical_device_dirty(adapter_id, short_address);
        true
    }

    pub(crate) fn apply_physical_runtime_short(
        &self,
        adapter_id: u8,
        commit: &PhysicalRuntimeCommit<'_>,
    ) -> RuntimeCommitOutcome {
        let mut g = self.write_inner();
        let Some(rec) = g.physical_devices.get_mut(&(adapter_id, commit.short_address)) else {
            return RuntimeCommitOutcome::NoRecord;
        };
        if !observation_supersedes(commit.observed_at_mono_ms, rec.runtime.observed_at_mono_ms) {
            return RuntimeCommitOutcome::Superseded;
        }
        let silent = !setpoint_states_a_value(commit.setpoint)
            && rec.runtime.observation_adds_nothing(commit.observation);
        let power_cycled = commit_physical_runtime(rec, commit);
        drop(g);
        if power_cycled {
            self.dirty.mark_physical_device_dirty(adapter_id, commit.short_address);
        }
        if silent {
            return RuntimeCommitOutcome::Unchanged;
        }
        RuntimeCommitOutcome::Committed
    }

    pub(crate) fn apply_level_transition_short(
        &self,
        adapter_id: u8,
        commit: &LevelTransitionCommit<'_>,
    ) -> RuntimeCommitOutcome {
        let mut g = self.write_inner();
        let Some(rec) = g.physical_devices.get_mut(&(adapter_id, commit.short_address)) else {
            return RuntimeCommitOutcome::NoRecord;
        };
        if !observation_supersedes(commit.observed_at_mono_ms, rec.runtime.observed_at_mono_ms) {
            return RuntimeCommitOutcome::Superseded;
        }
        let setpoint = match resolve_transition(commit.transition, rec) {
            TransitionOutcome::Level(level) => LightSetpoint {
                power: PowerState::for_level(level),
                level,
                color: None,
            },
            TransitionOutcome::Unchanged | TransitionOutcome::Unknown => LightSetpoint::default(),
        };
        let physical = PhysicalRuntimeCommit {
            short_address: commit.short_address,
            setpoint: &setpoint,
            observation: commit.observation,
            observed_at_mono_ms: commit.observed_at_mono_ms,
            entry_last_dapc_source: None,
            entry_source: commit.source,
        };
        let silent = !setpoint_states_a_value(&setpoint)
            && rec.runtime.observation_adds_nothing(commit.observation);
        let power_cycled = commit_physical_runtime(rec, &physical);
        drop(g);
        if power_cycled {
            self.dirty.mark_physical_device_dirty(adapter_id, commit.short_address);
        }
        if silent {
            return RuntimeCommitOutcome::Unchanged;
        }
        RuntimeCommitOutcome::Committed
    }

    pub(crate) fn runtime_state_payload_from_physical(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<(LightSetpoint, RuntimeObservation)> {
        let g = self.read_inner();
        let rec = g.physical_devices.get(&(adapter_id, short_address))?;
        Some((
            light_setpoint_from_physical_record(rec),
            runtime_observation_from_runtime_data(&rec.runtime),
        ))
    }

    pub(crate) fn physical_short_address_on_other_adapter(
        &self,
        exclude_adapter_id: u8,
        short_address: u8,
    ) -> bool {
        let g = self.read_inner();
        g.physical_devices
            .keys()
            .any(|(aid, sa)| *aid != exclude_adapter_id && *sa == short_address)
    }

    pub(crate) fn apply_address_change(
        &self,
        adapter_id: u8,
        old_short_address: u8,
        new_short_address: u8,
    ) -> bool {
        if old_short_address == new_short_address {
            return false;
        }
        let mut g = self.write_inner();
        if g.physical_devices
            .contains_key(&(adapter_id, new_short_address))
        {
            return false;
        }
        let Some(record) = g.physical_devices.remove(&(adapter_id, old_short_address)) else {
            return false;
        };
        g.physical_devices
            .insert((adapter_id, new_short_address), record);
        let mut rebound = false;
        for ((aid, _vl), lamp) in g.lamps.iter_mut() {
            if *aid == adapter_id && lamp.binding_short == Some(old_short_address) {
                lamp.binding_short = Some(new_short_address);
                rebound = true;
            }
        }
        drop(g);
        self.dirty.mark_physical_device_dirty(adapter_id, old_short_address);
        self.dirty.mark_physical_device_dirty(adapter_id, new_short_address);
        if rebound {
            self.dirty.mark_virtual_lamps_dirty(adapter_id);
        }
        true
    }

    pub(crate) fn apply_device_replacement(
        &self,
        adapter_id: u8,
        failed_short_address: u8,
        replacement_short_address: u8,
        restore_metadata_and_overrides: bool,
        restore_attributes: bool,
        restore_groups: bool,
        restore_scenes: bool,
    ) -> Option<(bool, bool, bool, bool)> {
        if failed_short_address == replacement_short_address {
            return None;
        }
        let mut g = self.write_inner();
        let mut failed = g
            .physical_devices
            .remove(&(adapter_id, failed_short_address))?;
        let Some(mut replacement) = g
            .physical_devices
            .remove(&(adapter_id, replacement_short_address))
        else {
            g.physical_devices
                .insert((adapter_id, failed_short_address), failed);
            return None;
        };

        let restored = move_configuration(
            &mut failed,
            &mut replacement,
            (
                restore_metadata_and_overrides,
                restore_attributes,
                restore_groups,
                restore_scenes,
            ),
        );

        g.physical_devices
            .insert((adapter_id, failed_short_address), replacement);
        for ((aid, _vl), lamp) in g.lamps.iter_mut() {
            if *aid == adapter_id && lamp.binding_short == Some(replacement_short_address) {
                lamp.binding_short = Some(failed_short_address);
            }
        }
        drop(g);
        self.dirty
            .mark_physical_device_dirty(adapter_id, failed_short_address);
        self.dirty
            .mark_physical_device_dirty(adapter_id, replacement_short_address);
        self.dirty.mark_virtual_lamps_dirty(adapter_id);
        Some(restored)
    }

    #[allow(clippy::too_many_arguments, reason = "mirrors the write-attributes event")]
    pub(crate) fn apply_attributes_written(
        &self,
        adapter_id: u8,
        short_address: u8,
        fade_time_ms: Option<u32>,
        fade_rate: Option<u8>,
        power_on_level: Option<u8>,
        system_failure_level: Option<u8>,
        extended_fade_time_ms: Option<u16>,
        tc_limits_mirek: (Option<u16>, Option<u16>),
        min_max_levels: (Option<u8>, Option<u8>),
        dimming_curve: Option<u8>,
    ) -> bool {
        let mut g = self.write_inner();
        let Some(rec) = g.physical_devices.get_mut(&(adapter_id, short_address)) else {
            return false;
        };
        let now = crate::runtime::registry::store::registry_unix_ms();
        let c102 = &mut rec.attributes.common102;
        let mut changed = merge_write_attr_opt(&mut c102.fade_time_ms, fade_time_ms, now);
        changed |= merge_write_attr_opt(&mut c102.fade_rate, fade_rate, now);
        changed |= merge_write_attr_opt(&mut c102.power_on_level, power_on_level, now);
        changed |= merge_write_attr_opt(&mut c102.system_failure_level, system_failure_level, now);
        changed |= merge_write_attr_opt(&mut c102.min_level, min_max_levels.0, now);
        changed |= merge_write_attr_opt(&mut c102.max_level, min_max_levels.1, now);
        changed |= merge_write_attr_opt(
            &mut rec.attributes.extended.fade_time_ms,
            extended_fade_time_ms,
            now,
        );
        if let (Some(coolest), Some(warmest)) = tc_limits_mirek {
            changed |= replace_changed(&mut rec.tc_coolest_mirek, Some(coolest));
            changed |= replace_changed(&mut rec.tc_warmest_mirek, Some(warmest));
        }
        changed |= merge_write_attr_opt(&mut rec.attributes.dt6_led.dimming_curve, dimming_curve, now);
        g.physical_devices_revision = g.physical_devices_revision.saturating_add(1);
        drop(g);
        if changed {
            self.dirty.mark_physical_device_dirty(adapter_id, short_address);
        }
        true
    }

    pub(crate) fn physical_device_to_view(
        &self,
        adapter_id: u8,
        short_address: u8,
        rec: &PhysicalDeviceRecord,
    ) -> PhysicalDeviceView {
        let dt_eff = rec.effective_device_type();
        let cm_eff = rec.effective_color_mode();
        let dt_src = override_source_str(rec.device_type_override.is_some());
        let cm_src = override_source_str(rec.color_mode_override.is_some());

        PhysicalDeviceView {
            adapter_id,
            short_address,
            random_address: rec.random_address,
            name: rec.name.as_str().to_string(),
            notes: rec.notes.as_ref().map(|n| n.as_str().to_string()),
            device_type_discovered: device_type_str(rec.device_type_discovered).to_string(),
            device_type_override: rec
                .device_type_override
                .map(|d| device_type_str(d).to_string()),
            device_type_effective: device_type_str(dt_eff).to_string(),
            device_type_source: dt_src.to_string(),
            supported_device_types: rec.supported_device_types.map(|set| set.iter().collect()),
            color_mode_discovered: color_mode_str(rec.color_mode_discovered).to_string(),
            color_mode_override: rec
                .color_mode_override
                .map(|c| color_mode_str(c).to_string()),
            color_mode_effective: color_mode_str(cm_eff).to_string(),
            color_mode_source: cm_src.to_string(),
            dt8_auto_activation_repair: rec.dt8_auto_activation_repair.unwrap_or(true),
            dt8_rgbwaf_control_assert: rec.dt8_rgbwaf_control_assert.unwrap_or(true),
            attributes: rec.attributes.clone(),
            energy: rec.energy.clone(),
            diagnostics: rec.diagnostics.clone(),
            bus_unit: rec.bus_unit.clone(),
            luminaire_info: luminaire_view(rec),
            memory_banks: memory_banks_view(&rec.memory_banks),
            state: physical_device_state_view(rec, cm_eff),
            capabilities: effective_capabilities_view(rec, cm_eff),
            color_temperature_range: ColorTemperatureRangeView::from_mirek(
                rec.tc_coolest_mirek,
                rec.tc_warmest_mirek,
            ),
        }
    }

    pub(crate) fn physical_device_to_summary_view(
        &self,
        short_address: u8,
        rec: &PhysicalDeviceRecord,
    ) -> PhysicalDeviceSummaryView {
        let cm_eff = rec.effective_color_mode();
        let identity = &rec.attributes.memory_identity;
        PhysicalDeviceSummaryView {
            short_address,
            random_address: rec.random_address,
            name: rec.name.as_str().to_string(),
            device_type_effective: device_type_str(rec.effective_device_type()),
            color_mode_effective: color_mode_str(cm_eff),
            capabilities: effective_capabilities_view(rec, cm_eff),
            state: physical_device_state_view(rec, cm_eff),
            groups_membership: rec.attributes.groups.membership.as_ref().map(|o| o.value),
            gtin: identity.gtin.as_ref().map(|o| o.value),
            identification_number: identity.identification_number.as_ref().map(|o| o.value),
        }
    }

    pub(crate) fn physical_device_to_core_view(
        &self,
        adapter_id: u8,
        short_address: u8,
        rec: &PhysicalDeviceRecord,
    ) -> PhysicalDeviceCoreView {
        let dt_eff = rec.effective_device_type();
        let cm_eff = rec.effective_color_mode();
        PhysicalDeviceCoreView {
            adapter_id,
            short_address,
            random_address: rec.random_address,
            name: rec.name.as_str().to_string(),
            notes: rec.notes.as_ref().map(|n| n.as_str().to_string()),
            device_type_discovered: device_type_str(rec.device_type_discovered),
            device_type_override: rec.device_type_override.map(device_type_str),
            device_type_effective: device_type_str(dt_eff),
            device_type_source: override_source_str(rec.device_type_override.is_some()),
            supported_device_types: rec.supported_device_types,
            extended_versions: rec.extended_versions,
            color_mode_discovered: color_mode_str(rec.color_mode_discovered),
            color_mode_override: rec.color_mode_override.map(color_mode_str),
            color_mode_effective: color_mode_str(cm_eff),
            color_mode_source: override_source_str(rec.color_mode_override.is_some()),
            dt8_auto_activation_repair: rec.dt8_auto_activation_repair.unwrap_or(true),
            dt8_rgbwaf_control_assert: rec.dt8_rgbwaf_control_assert.unwrap_or(true),
            state: physical_device_state_view(rec, cm_eff),
            capabilities: effective_capabilities_view(rec, cm_eff),
            color_temperature_range: ColorTemperatureRangeView::from_mirek(
                rec.tc_coolest_mirek,
                rec.tc_warmest_mirek,
            ),
        }
    }

    fn attribute_section(
        rec: &PhysicalDeviceRecord,
        kind: AttributeSectionKind,
    ) -> AttributeSectionView {
        let attrs = &rec.attributes;
        match kind {
            AttributeSectionKind::Common102 => {
                AttributeSectionView::Common102(Box::new(attrs.common102.clone()))
            }
            AttributeSectionKind::Dt6Led => {
                AttributeSectionView::Dt6Led(Box::new(attrs.dt6_led.clone()))
            }
            AttributeSectionKind::Dt8Color => {
                AttributeSectionView::Dt8Color(Box::new(attrs.dt8_color.clone()))
            }
            AttributeSectionKind::Extended => {
                AttributeSectionView::Extended(Box::new(attrs.extended.clone()))
            }
            AttributeSectionKind::Groups => {
                AttributeSectionView::Groups(Box::new(attrs.groups.clone()))
            }
            AttributeSectionKind::MemoryDiagnostics => {
                AttributeSectionView::MemoryDiagnostics(Box::new(rec.diagnostics.clone()))
            }
            AttributeSectionKind::MemoryEnergy => {
                AttributeSectionView::MemoryEnergy(Box::new(rec.energy.clone()))
            }
            AttributeSectionKind::MemoryBusUnit => {
                AttributeSectionView::MemoryBusUnit(Box::new(rec.bus_unit.clone()))
            }
            AttributeSectionKind::MemoryIdentity => {
                AttributeSectionView::MemoryIdentity(Box::new(attrs.memory_identity.clone()))
            }
            AttributeSectionKind::MemoryLuminaire => {
                AttributeSectionView::MemoryLuminaire(luminaire_view(rec).unwrap_or_default())
            }
            AttributeSectionKind::MemoryProfile => {
                AttributeSectionView::MemoryProfile(Box::new(attrs.memory_profile.clone()))
            }
            AttributeSectionKind::Scenes => {
                AttributeSectionView::Scenes(Box::new(attrs.scenes.clone()))
            }
        }
    }

    pub(crate) fn internal_known_physical_short_addresses(&self, adapter_id: u8) -> Vec<u8> {
        let g = self.read_inner();
        let mut shorts: Vec<u8> = g
            .physical_devices
            .keys()
            .filter_map(|(aid, sa)| (*aid == adapter_id).then_some(*sa))
            .collect();
        shorts.sort_unstable();
        shorts
    }
}

impl RegistryStore {
    pub fn physical_device_view(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceView> {
        let g = self.read_inner();
        let rec = g.physical_devices.get(&(adapter_id, short_address))?;
        Some(self.physical_device_to_view(adapter_id, short_address, rec))
    }
}

impl PhysicalDeviceReadPort for RegistryStore {
    fn physical_device_summary_view(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceSummaryView> {
        let g = self.read_inner();
        let rec = g.physical_devices.get(&(adapter_id, short_address))?;
        Some(self.physical_device_to_summary_view(short_address, rec))
    }

    fn physical_device_core_view(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<PhysicalDeviceCoreView> {
        let g = self.read_inner();
        let rec = g.physical_devices.get(&(adapter_id, short_address))?;
        Some(self.physical_device_to_core_view(adapter_id, short_address, rec))
    }

    fn physical_device_attribute_section(
        &self,
        adapter_id: u8,
        short_address: u8,
        kind: AttributeSectionKind,
    ) -> Option<AttributeSectionView> {
        let g = self.read_inner();
        let rec = g.physical_devices.get(&(adapter_id, short_address))?;
        Some(Self::attribute_section(rec, kind))
    }

    fn physical_device_memory_banks(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<Vec<MemoryBankSummaryView>> {
        let g = self.read_inner();
        let rec = g.physical_devices.get(&(adapter_id, short_address))?;
        Some(memory_banks_view(&rec.memory_banks))
    }

    fn physical_device_capabilities(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<CapabilityFlagsView> {
        let g = self.read_inner();
        let rec = g.physical_devices.get(&(adapter_id, short_address))?;
        Some(effective_capabilities_view(rec, rec.effective_color_mode()))
    }

    fn physical_device_supported_types(
        &self,
        adapter_id: u8,
        short_address: u8,
    ) -> Option<DeviceTypeSet> {
        let g = self.read_inner();
        g.physical_devices
            .get(&(adapter_id, short_address))?
            .supported_device_types
    }

    fn physical_device_exists(&self, adapter_id: u8, short_address: u8) -> bool {
        self.read_inner()
            .physical_devices
            .contains_key(&(adapter_id, short_address))
    }

    fn list_physical_device_short_addresses(&self, adapter_id: u8) -> Vec<u8> {
        self.internal_known_physical_short_addresses(adapter_id)
    }
}


fn move_configuration(
    failed: &mut PhysicalDeviceRecord,
    replacement: &mut PhysicalDeviceRecord,
    what: (bool, bool, bool, bool),
) -> (bool, bool, bool, bool) {
    let (metadata, attributes, groups, scenes) = what;
    let mut restored = (false, false, false, false);
    if metadata {
        replacement.name = failed.name.clone();
        replacement.notes = failed.notes.clone();
        replacement.device_type_override = failed.device_type_override;
        replacement.color_mode_override = failed.color_mode_override;
        restored.0 = true;
    }
    if attributes {
        core::mem::swap(&mut replacement.attributes, &mut failed.attributes);
        restored.1 = true;
    }
    restored.2 = groups && attributes;
    restored.3 = scenes && attributes;
    restored
}

#[cfg(test)]
mod tests {
    use super::super::store::RegistryStore;
    use super::DISCOVERY_EVICT_MISS_THRESHOLD;

    const VERIFIED_SHORT: u8 = 0;
    const PHANTOM_SHORT: u8 = 9;
    const GOLDEN_RANDOM: u32 = 0x005C_1DC2;

    fn apply_progress(store: &RegistryStore, short: u8, random_address: Option<u32>) -> bool {
        store.seed_discovered_dt8(short, random_address)
    }

    fn known_shorts(store: &RegistryStore) -> Vec<u8> {
        store.internal_known_physical_short_addresses(0)
    }

    #[test]
    fn discovery_progress_rejects_out_of_range_short_address() {
        let store = RegistryStore::with_adapter_count(1);
        assert!(!apply_progress(&store, 64, Some(GOLDEN_RANDOM)));
        assert!(!apply_progress(&store, u8::MAX, None));
        assert!(known_shorts(&store).is_empty());
    }

    #[test]
    fn refresh_progress_without_random_address_keeps_verified_evidence() {
        let store = RegistryStore::with_adapter_count(1);
        assert!(apply_progress(&store, VERIFIED_SHORT, Some(GOLDEN_RANDOM)));
        assert!(apply_progress(&store, VERIFIED_SHORT, None));
        let g = store.inner.read().expect("registry lock");
        let record = &g.physical_devices[&(0, VERIFIED_SHORT)];
        assert_eq!(record.random_address, Some(GOLDEN_RANDOM));
    }

    #[test]
    fn reconcile_evicts_never_verified_record_after_miss_threshold() {
        let store = RegistryStore::with_adapter_count(1);
        assert!(apply_progress(&store, VERIFIED_SHORT, Some(GOLDEN_RANDOM)));
        assert!(apply_progress(&store, PHANTOM_SHORT, None));

        for miss in 1..DISCOVERY_EVICT_MISS_THRESHOLD {
            let evicted = store.reconcile_discovery_scan(0, 0);
            assert!(evicted.is_empty(), "no eviction after {miss} misses");
            assert_eq!(known_shorts(&store), vec![VERIFIED_SHORT, PHANTOM_SHORT]);
        }
        let evicted = store.reconcile_discovery_scan(0, 0);
        assert_eq!(evicted, vec![PHANTOM_SHORT]);
        assert_eq!(known_shorts(&store), vec![VERIFIED_SHORT]);
        assert!(store.reconcile_discovery_scan(0, 0).is_empty());
        assert_eq!(known_shorts(&store), vec![VERIFIED_SHORT]);
    }

    #[test]
    fn reconcile_confirmation_resets_the_miss_count() {
        let store = RegistryStore::with_adapter_count(1);
        assert!(apply_progress(&store, PHANTOM_SHORT, None));

        for _ in 1..DISCOVERY_EVICT_MISS_THRESHOLD {
            assert!(store.reconcile_discovery_scan(0, 0).is_empty());
        }
        assert!(store
            .reconcile_discovery_scan(0, 1u64 << PHANTOM_SHORT)
            .is_empty());
        for _ in 1..DISCOVERY_EVICT_MISS_THRESHOLD {
            assert!(store.reconcile_discovery_scan(0, 0).is_empty());
        }
        assert_eq!(store.reconcile_discovery_scan(0, 0), vec![PHANTOM_SHORT]);
    }
}

#[cfg(test)]
mod issue37_dirty_gating_tests {
    use super::super::store::RegistryStore;
    use dali2rust_contracts::msg::{
        ColorMode, DaliAttributeReadChunk as Chunk, DeviceTypeSet, Dt6ReadSnapshot,
        RuntimeObservation,
    };

    const SHORT: u8 = 3;
    const SEED_RANDOM: u32 = 0x1234;

    fn seeded_store() -> RegistryStore {
        let store = RegistryStore::with_adapter_count(1);
        assert!(store.seed_discovered_dt8(SHORT, Some(SEED_RANDOM)));
        let _ = store.dirty.take_physical_devices_dirty_for(0);
        store
    }

    fn c102_two_fields() -> Chunk {
        Chunk::Common102 {
            version: Some(2),
            device_type: None,
            physical_minimum: None,
            min_level: Some(10),
            max_level: None,
            power_on_level: None,
            system_failure_level: None,
            fade_time_ms: None,
            fade_rate: None,
            supported_device_types: None,
            light_source_type: None,
            light_source_types: None,
        }
    }

    fn c102_declaring(types: &[u8]) -> Chunk {
        let mut set = DeviceTypeSet::default();
        for device_type in types {
            assert!(set.insert(*device_type));
        }
        Chunk::Common102 {
            version: None,
            device_type: None,
            physical_minimum: None,
            min_level: None,
            max_level: None,
            power_on_level: None,
            system_failure_level: None,
            fade_time_ms: None,
            fade_rate: None,
            supported_device_types: Some(set),
            light_source_type: None,
            light_source_types: None,
        }
    }

    #[test]
    fn an_attribute_read_adds_declared_device_types_and_never_removes_them() {
        let store = seeded_store();
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &c102_declaring(&[0, 8])));
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &c102_declaring(&[0])));

        let g = store.inner.read().expect("registry lock");
        let declared = g.physical_devices[&(0, SHORT)]
            .supported_device_types
            .expect("declared set");
        assert!(declared.contains(8), "the narrower read erased the probe's 8");
        assert!(declared.contains(0));
    }

    #[test]
    fn an_unfinished_enumeration_leaves_the_stored_declared_set_alone() {
        let store = seeded_store();
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &c102_declaring(&[6])));
        let _ = store.dirty.take_physical_devices_dirty_for(0);

        store.apply_physical_device_attribute_chunk(0, SHORT, &c102_two_fields());
        let g = store.inner.read().expect("registry lock");
        let declared = g.physical_devices[&(0, SHORT)]
            .supported_device_types
            .expect("declared set");
        assert!(declared.iter().eq([6]));
    }

    fn c102_fade(fade_time_ms: u32) -> Chunk {
        Chunk::Common102 {
            version: None,
            device_type: None,
            physical_minimum: None,
            min_level: None,
            max_level: None,
            power_on_level: None,
            system_failure_level: None,
            fade_time_ms: Some(fade_time_ms),
            fade_rate: None,
            supported_device_types: None,
            light_source_type: None,
            light_source_types: None,
        }
    }

    #[test]
    fn identical_attribute_chunk_reapply_leaves_the_slice_clean() {
        let store = seeded_store();
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &c102_two_fields()));
        assert!(store.dirty.take_physical_devices_dirty_for(0));
        let first_read_ms = {
            let g = store.inner.read().expect("registry lock");
            let c102 = &g.physical_devices[&(0, SHORT)].attributes.common102;
            c102.version.as_ref().expect("merged").last_read_ms
        };

        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &c102_two_fields()));
        assert!(
            !store.dirty.take_physical_devices_dirty_for(0),
            "an identical re-read must not re-dirty the slice"
        );
        let g = store.inner.read().expect("registry lock");
        let c102 = &g.physical_devices[&(0, SHORT)].attributes.common102;
        assert_eq!(c102.version.as_ref().expect("kept").value, 2);
        assert_eq!(c102.min_level.as_ref().expect("kept").value, 10);
        assert!(c102.version.as_ref().expect("kept").last_read_ms >= first_read_ms);
    }

    #[test]
    fn runtime_status_chunk_never_dirties() {
        let store = seeded_store();
        let chunk = Chunk::RuntimeStatus {
            setpoint: None,
            observation: RuntimeObservation::default(),
            read_started_mono_ms: 0,
        };
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &chunk));
        assert!(
            !store.dirty.take_physical_devices_dirty_for(0),
            "the runtime section writes nothing persisted and must not dirty"
        );
    }

    #[test]
    fn identical_chunks_of_every_family_reapply_clean() {
        for chunk in chunk_family_fixtures() {
            let store = seeded_store();
            assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &chunk));
            assert!(
            store.dirty.take_physical_devices_dirty_for(0),
                "first apply of {chunk:?} must dirty"
            );
            assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &chunk));
            assert!(
            !store.dirty.take_physical_devices_dirty_for(0),
                "identical re-apply of {chunk:?} must stay clean"
            );
        }
    }

    fn chunk_family_fixtures() -> Vec<Chunk> {
        vec![
            Chunk::Identity {
                random_address: 0x9999,
            },
            c102_two_fields(),
            Chunk::Dt8Color {
                color_mode: ColorMode::Cct,
                xy_capable: false,
                tc_capable: true,
                rgb_capable: false,
                color_type: Some(32),
                color_value_0: Some(370),
                color_value_1: None,
                color_value_2: None,
                tc_coolest_mirek: Some(153),
                tc_warmest_mirek: Some(370),
                gear_features: Some(0x01),
                rgbwaf_capable: false,
                rgbwaf_control: None,
            },
            Chunk::Groups {
                membership: Some(0x0202),
            },
            Chunk::Scenes { levels: Some([7; 16]) },
            Chunk::Dt6Led {
                snapshot: Dt6ReadSnapshot {
                    gear_type: Some(1),
                    dimming_curve: Some(0),
                    ..Dt6ReadSnapshot::default()
                },
            },
            Chunk::Extended {
                fade_time_ms: Some(700),
                version_number: Some(2),
            },
        ]
    }

    #[test]
    fn groups_membership_clear_dirties_once_then_stays_clean() {
        let store = seeded_store();
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &Chunk::Groups { membership: None }));
        assert!(
            !store.dirty.take_physical_devices_dirty_for(0),
            "clearing an already-empty membership is a no-op"
        );
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &Chunk::Groups { membership: Some(3) }));
        assert!(store.dirty.take_physical_devices_dirty_for(0));
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &Chunk::Groups { membership: None }));
        assert!(
            store.dirty.take_physical_devices_dirty_for(0),
            "Some -> None is a real change and dirties once"
        );
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &Chunk::Groups { membership: None }));
        assert!(!store.dirty.take_physical_devices_dirty_for(0));
    }

    #[test]
    fn attribute_source_transitions_count_as_change() {
        let store = seeded_store();
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &c102_fade(700)));
        assert!(store.dirty.take_physical_devices_dirty_for(0));

        assert!(store.apply_attributes_written(0, SHORT, Some(700), None, None, None, None, (None, None), (None, None), None));
        assert!(
            store.dirty.take_physical_devices_dirty_for(0),
            "Readback -> WriteConfirmed at the same value is a change"
        );
        {
            let g = store.inner.read().expect("registry lock");
            let slot = g.physical_devices[&(0, SHORT)]
                .attributes
                .common102
                .fade_time_ms
                .clone()
                .expect("slot");
            assert!(slot.last_read_ms.is_some(), "write keeps the read stamp");
            assert!(slot.last_write_confirmed_ms.is_some());
        }

        assert!(store.apply_attributes_written(0, SHORT, Some(700), None, None, None, None, (None, None), (None, None), None));
        assert!(
            !store.dirty.take_physical_devices_dirty_for(0),
            "a byte-identical re-confirmation is the ISSUE-37 no-op class"
        );

        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &c102_fade(700)));
        assert!(
            store.dirty.take_physical_devices_dirty_for(0),
            "WriteConfirmed -> Readback at the same value is a change"
        );
        {
            let g = store.inner.read().expect("registry lock");
            let slot = g.physical_devices[&(0, SHORT)]
                .attributes
                .common102
                .fade_time_ms
                .clone()
                .expect("slot");
            assert!(
                slot.last_write_confirmed_ms.is_some(),
                "readback keeps the write-confirmation stamp"
            );
        }
        assert!(store.apply_physical_device_attribute_chunk(0, SHORT, &c102_fade(700)));
        assert!(!store.dirty.take_physical_devices_dirty_for(0));
    }
}

#[cfg(test)]
mod stack_footprint_tests {
    use super::*;

    #[test]
    fn rekeying_a_record_moves_a_pointer_not_the_record() {
        type Stored = PsramBox<PhysicalDeviceRecord>;
        assert_eq!(
            core::mem::size_of::<Stored>(),
            core::mem::size_of::<usize>(),
            "the map's value must stay pointer-sized; a {} B record moving through \
             the worker stack is what the boxing exists to prevent",
            core::mem::size_of::<PhysicalDeviceRecord>()
        );
    }

    #[test]
    fn the_placed_record_stays_within_its_ceiling() {
        const CEILING: usize = 8_192;
        let size = core::mem::size_of::<PhysicalDeviceRecord>();
        assert!(
            size <= CEILING,
            "PhysicalDeviceRecord is {size} B, ceiling {CEILING} B"
        );
    }

    #[test]
    fn no_inline_section_temporary_exceeds_the_budget() {
        const MAX_INLINE_INIT_TEMP_BYTES: usize = 2_048;
        for (name, size) in PhysicalDeviceRecord::INLINE_INIT_TEMPORARIES {
            assert!(
                *size <= MAX_INLINE_INIT_TEMP_BYTES,
                "{name} is {size} B: give it a `default_in_place` and move it to \
                 the `placed` group, or the temporary lands on the worker stack"
            );
        }
    }

    #[test]
    fn the_budget_list_covers_every_field_written_by_value() {
        const PLACED: usize = 2;
        assert_eq!(
            PhysicalDeviceRecord::INLINE_INIT_TEMPORARIES.len() + PLACED,
            29,
            "the field list and the record have diverged; both come from \
             physical_device_record_fields!, so this means a field was added \
             to the struct alone"
        );
    }

    const _: fn(&PhysicalDeviceRecord) -> Option<&PsramBox<MemoryLuminaireAttributesView>> =
        |rec| rec.luminaire_info.as_ref();

    #[test]
    fn placement_and_by_value_construction_agree() {
        let mut place: MaybeUninit<PhysicalDeviceRecord> = MaybeUninit::uninit();
        PhysicalDeviceRecord::init_empty(&mut place);
        // SAFETY: `init_empty` initialises every field of the place.
        let placed = unsafe { place.assume_init() };
        assert_eq!(
            format!("{placed:?}"),
            format!("{:?}", PhysicalDeviceRecord::empty(0))
        );
    }

    #[test]
    fn new_boxed_places_the_same_record() {
        let boxed = PhysicalDeviceRecord::new_boxed();
        assert_eq!(
            format!("{:?}", *boxed),
            format!("{:?}", PhysicalDeviceRecord::empty(0))
        );
    }
}

#[cfg(test)]
mod power_cycle_tests {
    use super::*;

    fn status_read(level: u8, status: u8) -> (LightSetpoint, RuntimeObservation) {
        let setpoint = LightSetpoint { power: PowerState::for_level(level), level, color: None };
        let observation = RuntimeObservation {
            status_flags: Some(dali2rust_domain::dali::status::decode(status)),
            ..RuntimeObservation::default()
        };
        (setpoint, observation)
    }

    fn commit_status_read(rec: &mut super::PhysicalDeviceRecord, level: u8, status: u8) -> bool {
        let (setpoint, observation) = status_read(level, status);
        super::commit_physical_runtime(
            rec,
            &super::PhysicalRuntimeCommit {
                short_address: 0,
                setpoint: &setpoint,
                observation: &observation,
                observed_at_mono_ms: None,
                entry_last_dapc_source: None,
                entry_source: RuntimeSource::Readback,
            },
        )
    }

    #[test]
    fn a_power_cycle_puts_last_active_level_back_to_max_level_and_the_power_on_level_leaves_it() {
        const STATUS_LAMP_ON: u8 = 0x04;
        const STATUS_LAMP_ON_AFTER_POWER_CYCLE: u8 = 0x84;
        const MAX_LEVEL: u8 = 200;
        let mut rec = super::PhysicalDeviceRecord::empty(0);
        rec.attributes.common102.max_level = Some(ObservedValue {
            value: MAX_LEVEL,
            source: dali2rust_domain::registry::AttributeSource::Readback,
            last_read_ms: Some(1),
            last_write_confirmed_ms: None,
        });
        assert!(!commit_status_read(&mut rec, 90, STATUS_LAMP_ON));
        assert_eq!(rec.last_active_level, Some(90));
        assert!(commit_status_read(&mut rec, 150, STATUS_LAMP_ON_AFTER_POWER_CYCLE));
        assert_eq!(rec.runtime_level, Some(150), "the power-on level is what the lamp shows");
        assert_eq!(
            rec.last_active_level,
            Some(MAX_LEVEL),
            "IEC 62386-102 Table 14 powers lastActiveLevel up at maxLevel, and §9.4 keeps the \
             power-on initialisation from moving it"
        );
        assert!(
            !commit_status_read(&mut rec, 150, STATUS_LAMP_ON_AFTER_POWER_CYCLE),
            "the bit stays set until a level command: a second read is not a second power cycle"
        );
    }
}
