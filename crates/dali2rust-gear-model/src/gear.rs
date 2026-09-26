use dali2rust_domain::dali::banks::part251::LuminaireFormat;
use dali2rust_domain::dali::banks::{BUS_UNIT_CONFIGURATION_OFFSET, IMPLEMENTED_PARTS_OFFSET};

pub const DEVICE_TYPE_LUMINAIRE_INFO: u8 = 50;
use dali2rust_domain::dali::devices::dt8_color::{
    gear_features_automatic_activation, COLOUR_STATUS_TC_OUT_OF_RANGE, COLOUR_TYPE_BYTE_MASK,
    COLOUR_TYPE_BYTE_RGBWAF, COLOUR_TYPE_BYTE_TC, COLOUR_TYPE_BYTE_XY, COLOUR_VALUE_FREECOLOUR,
    COLOUR_VALUE_LEVEL_MASK, COLOUR_VALUE_MASK, COLOUR_VALUE_NUMBER_OF_PRIMARIES,
    COLOUR_VALUE_RED, COLOUR_VALUE_REPORT_COLOUR_TYPE, COLOUR_VALUE_REPORT_FREECOLOUR,
    COLOUR_VALUE_REPORT_RED, COLOUR_VALUE_REPORT_RGBWAF_CONTROL, COLOUR_VALUE_REPORT_TC,
    COLOUR_VALUE_REPORT_X, COLOUR_VALUE_REPORT_Y,
    COLOUR_VALUE_RGBWAF_CONTROL, COLOUR_VALUE_TC, COLOUR_VALUE_TC_COOLEST,
    COLOUR_VALUE_TC_PHYSICAL_COOLEST, COLOUR_VALUE_TC_PHYSICAL_WARMEST, COLOUR_VALUE_TC_WARMEST,
    COLOUR_VALUE_TEMPORARY_COLOUR_TYPE, COLOUR_VALUE_TEMPORARY_RGBWAF_CONTROL,
    COLOUR_VALUE_TEMPORARY_TC, COLOUR_VALUE_TEMPORARY_X, COLOUR_VALUE_TEMPORARY_Y, COLOUR_VALUE_X,
    COLOUR_VALUE_Y, GEAR_FEATURES_POWER_UP_DEFAULT, RGBWAF_CONTROL_ALL_CHANNELS,
    RGBWAF_CONTROL_MASK, RGBWAF_CONTROL_NORMALISED, RGBWAF_CONTROL_POWER_UP_DEFAULT,
    RGBWAF_CONTROL_TYPE_MASK, TC_LIMIT_SELECTOR_COOLEST, TC_LIMIT_SELECTOR_PHYSICAL_COOLEST,
    TC_LIMIT_SELECTOR_PHYSICAL_WARMEST, TC_LIMIT_SELECTOR_WARMEST,
};
use dali2rust_domain::dali::devices::dt6_led::{decode_failure_status, FailureStatusBits};
use dali2rust_domain::dali::devices::DeviceType;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_platform::dali::TransferOutcome;

use crate::luminaire::LuminaireBank;

pub const DALI_YES: u8 = 0xFF;
pub const SCENE_UNSET: u8 = 0xFF;
pub const DAPC_MASK_LEVEL: u8 = 0xFF;
pub const DEVICE_TYPE_LIST_END: u8 = 254;
pub const DEVICE_TYPE_MULTIPLE: u8 = 255;

pub use dali2rust_domain::dali::status::{
    STATUS_FADE_RUNNING, STATUS_GEAR_FAILURE, STATUS_LAMP_FAILURE, STATUS_LAMP_ON,
    STATUS_LIMIT_ERROR, STATUS_MISSING_SHORT_ADDRESS, STATUS_POWER_CYCLE_SEEN, STATUS_RESET_STATE,
};

pub(crate) use dali2rust_domain::dali::devices::dt8_color::{
    COLOUR_STATUS_RGBWAF_ACTIVE as DT8_STATUS_RGB_ACTIVE,
    COLOUR_STATUS_TC_ACTIVE as DT8_STATUS_TC_ACTIVE,
    COLOUR_STATUS_XY_ACTIVE as DT8_STATUS_XY_ACTIVE,
};
const DT8_FEATURE_XY: u8 = 0x01;
const DT8_FEATURE_TC: u8 = 0x02;
const DT8_FEATURE_RGBWAF_CHANNEL_SHIFT: u8 = 5;
pub const DEFAULT_RGBWAF_CHANNELS: u8 = 3;

const UP_DOWN_LEVEL_STEP: u8 = 8;
pub const DEFAULT_MIN_LEVEL: u8 = 1;
pub const DEFAULT_MAX_LEVEL: u8 = 254;
pub const DEFAULT_POWER_ON_LEVEL: u8 = 254;
pub const DEFAULT_SYSTEM_FAILURE_LEVEL: u8 = 254;
const DEFAULT_FADE_TIME: u8 = 0;
const DEFAULT_FADE_RATE: u8 = 7;
const PHYSICAL_MINIMUM: u8 = 1;
// IEC 62386-102 §4.2, §11.5.10
const VERSION_NUMBER_102: u8 = 0x08;
const VERSION_NUMBER_101: u8 = 0x08;
const LIGHT_SOURCE_LED: u8 = 6;
pub const SHORT_ADDRESS_MASK: u8 = 0x3F;
use dali2rust_domain::registry::GROUP_COUNT;
pub const SCENE_COUNT: usize = dali2rust_domain::registry::SCENE_COUNT as usize;
pub(crate) const MEMORY_BANK_IDENTITY: u8 = 0;
pub(crate) const MEMORY_BANK_PROFILE: u8 = 1;
pub(crate) const BANK0_LEN: usize = 0x1D;
pub(crate) const BANK0_BASE_LAST_OFFSET: u8 = 0x1A;
pub(crate) const BANK0_EXTENDED_LAST_OFFSET: u8 = 0x1C;
pub(crate) const BANK1_LEN: usize = 0x11;
pub(crate) const BANK0_LAST_BANK: u8 = 1;
pub(crate) const BANK1_UNLOCKED: u8 = 0x55;

pub const TC_COOLEST_MIREK: u16 = 153;
pub const TC_WARMEST_MIREK: u16 = 400;

const XY_STEP: u16 = 256;
const XY_COORDINATE_MAX: u16 = 0xFFFE;

#[derive(Debug, Clone)]
pub struct GearSpec {
    pub short_address: Option<u8>,
    pub random_address: u32,
    pub device_types: Vec<u8>,
    pub tc_capable: bool,
    pub xy_capable: bool,
    pub rgbwaf_channels: u8,
    pub tc_coolest_mirek: u16,
    pub tc_warmest_mirek: u16,
    pub phm: u8,
    pub energy_reporting: bool,
    pub diagnostics_reporting: bool,
    pub bus_unit_extension: bool,
    pub luminaire_format: Option<LuminaireFormat>,
}

impl GearSpec {
    pub fn dt6(short_address: Option<u8>, random_address: u32) -> Self {
        Self {
            short_address,
            random_address,
            device_types: vec![DeviceType::Led.code()],
            tc_capable: false,
            xy_capable: false,
            rgbwaf_channels: 0,
            tc_coolest_mirek: TC_COOLEST_MIREK,
            tc_warmest_mirek: TC_WARMEST_MIREK,
            phm: PHYSICAL_MINIMUM,
            energy_reporting: false,
            diagnostics_reporting: false,
            bus_unit_extension: false,
            luminaire_format: None,
        }
    }

    pub fn dt8(
        short_address: Option<u8>,
        random_address: u32,
        caps: (bool, bool, bool),
        tc_range: (u16, u16),
    ) -> Self {
        let (tc_capable, xy_capable, rgb_capable) = caps;
        Self {
            short_address,
            random_address,
            device_types: vec![DeviceType::Color.code()],
            tc_capable,
            xy_capable,
            rgbwaf_channels: if rgb_capable { DEFAULT_RGBWAF_CHANNELS } else { 0 },
            tc_coolest_mirek: tc_range.0,
            tc_warmest_mirek: tc_range.1,
            phm: PHYSICAL_MINIMUM,
            energy_reporting: false,
            diagnostics_reporting: false,
            bus_unit_extension: false,
            luminaire_format: None,
        }
    }
}

impl GearSpec {
    pub fn rgb_capable(&self) -> bool {
        self.rgbwaf_channels > 0
    }

    pub fn with_metering(mut self, energy: bool, diagnostics: bool) -> Self {
        self.energy_reporting = energy;
        self.diagnostics_reporting = diagnostics;
        if energy && !self.device_types.contains(&DEVICE_TYPE_ENERGY) {
            self.device_types.push(DEVICE_TYPE_ENERGY);
        }
        if diagnostics && !self.device_types.contains(&DEVICE_TYPE_DIAGNOSTICS) {
            self.device_types.push(DEVICE_TYPE_DIAGNOSTICS);
        }
        self
    }
}

pub use dali2rust_domain::dali::banks::{DEVICE_TYPE_DIAGNOSTICS, DEVICE_TYPE_ENERGY};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    None,
    Cct,
    Xy,
    Rgb,
}

#[derive(Debug, Clone, Copy)]
pub struct Rgbwaf {
    pub pending_levels: [u8; 6],
    pub pending_control: u8,
    pub levels: [u8; 6],
    pub control: u8,
}

impl Rgbwaf {
    pub fn at_power_up() -> Self {
        Self {
            pending_levels: [0; 6],
            pending_control: RGBWAF_CONTROL_MASK,
            levels: [0; 6],
            control: RGBWAF_CONTROL_POWER_UP_DEFAULT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColourSetting {
    pub colour_type: u8,
    pub x: u16,
    pub y: u16,
    pub tc: u16,
    pub levels: [u8; 6],
    pub control: u8,
}

impl ColourSetting {
    pub const fn mask() -> Self {
        Self {
            colour_type: COLOUR_TYPE_BYTE_MASK,
            x: COLOUR_VALUE_MASK,
            y: COLOUR_VALUE_MASK,
            tc: COLOUR_VALUE_MASK,
            levels: [COLOUR_VALUE_LEVEL_MASK; 6],
            control: RGBWAF_CONTROL_MASK,
        }
    }
}

pub(crate) fn colour_type_byte(mode: ColorMode) -> u8 {
    match mode {
        ColorMode::None => COLOUR_TYPE_BYTE_MASK,
        ColorMode::Xy => COLOUR_TYPE_BYTE_XY,
        ColorMode::Cct => COLOUR_TYPE_BYTE_TC,
        ColorMode::Rgb => COLOUR_TYPE_BYTE_RGBWAF,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FaultInjection {
    pub drop_answer_permille: u16,
    pub forced_status: u8,
    pub failure_status: u8,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Heard {
    pub executes: bool,
    pub completed_pair: bool,
    pub split: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingRandom {
    pub(crate) address: u32,
    pub(crate) ready_at_ms: u64,
}

#[derive(Debug)]
pub struct Gear {
    pub spec: GearSpec,
    pub enabled: bool,
    pub level: u8,
    pub last_active_level: u8,
    pub min_level: u8,
    pub max_level: u8,
    pub power_on_level: u8,
    pub system_failure_level: u8,
    pub fade_time: u8,
    pub fade_rate: u8,
    pub extended_fade_time: u8,
    pub groups: u16,
    pub scenes: [u8; SCENE_COUNT],
    pub lamp_failure: bool,
    pub lamp_starting: bool,
    pub faults: FaultInjection,
    pub reset_state: bool,
    pub power_cycle_seen: bool,
    pub initialise: bool,
    pub withdrawn: bool,
    pub identifying: bool,
    pub dimming_curve: u8,
    device_type_walk: Option<usize>,
    pub limit_error: bool,
    pending_repeat: Option<u16>,
    pub(crate) pending_random: Option<PendingRandom>,
    pub(crate) executes_now: bool,
    pub color_mode: ColorMode,
    pub pending_mode: ColorMode,
    pub pending_x: u16,
    pub pending_y: u16,
    pub pending_ct: u16,
    pub rgbwaf: Rgbwaf,
    pub color_x: u16,
    pub color_y: u16,
    pub color_ct: u16,
    tc_out_of_range: bool,
    pub tc_coolest_limit: u16,
    pub tc_warmest_limit: u16,
    pub scene_colours: [ColourSetting; SCENE_COUNT],
    pub report: ColourSetting,
    pub gear_features: u8,
    bank0: [u8; BANK0_LEN],
    bank1: [u8; BANK1_LEN],
    luminaire: Option<Box<LuminaireBank>>,
    pub metering: Option<Box<crate::metering::MeteringBanks>>,
}

fn build_bank0(spec: &GearSpec) -> [u8; BANK0_LEN] {
    let mut b = [0u8; BANK0_LEN];
    b[0] = if spec.bus_unit_extension {
        BANK0_EXTENDED_LAST_OFFSET
    } else {
        BANK0_BASE_LAST_OFFSET
    };
    b[2] = BANK0_LAST_BANK;
    let gtin: u64 = 0x4000_0000_0000u64 | u64::from(spec.random_address);
    b[3..9].copy_from_slice(&gtin.to_be_bytes()[2..8]);
    b[9] = 1;
    b[10] = 4;
    let serial: u64 = 0x5EED_0000u64 + u64::from(spec.random_address);
    b[11..19].copy_from_slice(&serial.to_be_bytes());
    b[19] = 2;
    b[20] = 0;
    b[21] = VERSION_NUMBER_101;
    b[22] = VERSION_NUMBER_102;
    b[23] = 0;
    b[24] = 0;
    b[25] = 1;
    b[26] = 0;
    if spec.bus_unit_extension {
        b[usize::from(BUS_UNIT_CONFIGURATION_OFFSET)] = 0;
        b[usize::from(IMPLEMENTED_PARTS_OFFSET)] = 0b0000_0001;
    }
    b
}

impl Gear {
    fn identity_location(&self, offset: u8) -> Option<u8> {
        if offset > self.bank0[0] {
            return None;
        }
        // IEC 62386-102 §9.10.4, Table 9
        if offset == BANK0_RESERVED_OFFSET {
            return None;
        }
        let raw = self.bank0.get(usize::from(offset)).copied()?;
        if usize::from(offset) != BANK0_LAST_BANK_OFFSET {
            return Some(raw);
        }
        Some(match self.metering.as_ref() {
            Some(m) => m.last_accessible_bank(raw),
            None => raw,
        })
    }
}

impl Gear {
    fn profile_location(&self, offset: u8) -> Option<u8> {
        if usize::from(offset) == BANK1_LAST_OFFSET_OFFSET {
            return Some(match self.luminaire.as_ref() {
                Some(l) => l.last_accessible(),
                None => (BANK1_LEN - 1) as u8,
            });
        }
        match self.luminaire.as_ref() {
            Some(l) if offset >= crate::luminaire::LUMINAIRE_FIRST_OFFSET => l.location(offset),
            _ => self.bank1.get(usize::from(offset)).copied(),
        }
    }
}

const BANK1_LAST_OFFSET_OFFSET: usize = 0x00;

const BANK0_LAST_BANK_OFFSET: usize = 0x02;

const BANK0_RESERVED_OFFSET: u8 = 0x01;

const OPERATING_MODE_STANDARD: u8 = 0x00;
const MANUFACTURER_SPECIFIC_MODE_MIN: u8 = 0x80;

const fn is_manufacturer_specific_mode(mode: u8) -> bool {
    mode >= MANUFACTURER_SPECIFIC_MODE_MIN
}

fn build_bank1(spec: &GearSpec) -> [u8; BANK1_LEN] {
    let mut b = [0u8; BANK1_LEN];
    b[0] = (BANK1_LEN - 1) as u8;
    b[2] = BANK1_UNLOCKED;
    let oem_gtin: u64 = 0x0EE0_0000_0000u64 | u64::from(spec.random_address);
    b[3..9].copy_from_slice(&oem_gtin.to_be_bytes()[2..8]);
    let oem_id: u64 = u64::from(spec.random_address) << 8;
    b[9..17].copy_from_slice(&oem_id.to_be_bytes());
    b
}

fn build_luminaire(spec: &GearSpec) -> Option<Box<LuminaireBank>> {
    spec.luminaire_format.map(|format| {
        Box::new(LuminaireBank::new(
            spec.random_address,
            format,
            spec.tc_capable,
        ))
    })
}

fn build_metering(spec: &GearSpec) -> Option<Box<crate::metering::MeteringBanks>> {
    (spec.energy_reporting || spec.diagnostics_reporting).then(|| {
        Box::new(crate::metering::MeteringBanks::new(
            spec.random_address,
            spec.energy_reporting,
            spec.diagnostics_reporting,
        ))
    })
}

impl Gear {
    pub fn new(spec: GearSpec) -> Self {
        Self {
            min_level: spec.phm,
            tc_coolest_limit: spec.tc_coolest_mirek,
            tc_warmest_limit: spec.tc_warmest_mirek,
            bank0: build_bank0(&spec),
            bank1: build_bank1(&spec),
            luminaire: build_luminaire(&spec),
            metering: build_metering(&spec),
            spec,
            enabled: true,
            level: 0,
            last_active_level: DEFAULT_MAX_LEVEL,
            max_level: DEFAULT_MAX_LEVEL,
            power_on_level: DEFAULT_POWER_ON_LEVEL,
            system_failure_level: DEFAULT_SYSTEM_FAILURE_LEVEL,
            fade_time: DEFAULT_FADE_TIME,
            fade_rate: DEFAULT_FADE_RATE,
            extended_fade_time: 0,
            groups: 0,
            scenes: [SCENE_UNSET; SCENE_COUNT],
            lamp_failure: false,
            lamp_starting: false,
            faults: FaultInjection::default(),
            reset_state: true,
            power_cycle_seen: true,
            initialise: false, withdrawn: false, identifying: false, dimming_curve: 0,
            device_type_walk: None,
            limit_error: false,
            pending_repeat: None,
            pending_random: None,
            executes_now: false,
            rgbwaf: Rgbwaf::at_power_up(),
            color_mode: ColorMode::None,
            pending_mode: ColorMode::None,
            pending_x: 0, pending_y: 0, pending_ct: 0,
            color_x: 0, color_y: 0, color_ct: 0,
            tc_out_of_range: false,
            scene_colours: [ColourSetting::mask(); SCENE_COUNT],
            report: ColourSetting::mask(),
            gear_features: GEAR_FEATURES_POWER_UP_DEFAULT,
        }
    }

    pub fn set_metering(&mut self, energy: bool, diagnostics: bool) {
        self.spec.energy_reporting = energy;
        self.spec.diagnostics_reporting = diagnostics;
        self.spec
            .device_types
            .retain(|t| *t != DEVICE_TYPE_ENERGY && *t != DEVICE_TYPE_DIAGNOSTICS);
        if energy {
            self.spec.device_types.push(DEVICE_TYPE_ENERGY);
        }
        if diagnostics {
            self.spec.device_types.push(DEVICE_TYPE_DIAGNOSTICS);
        }
        self.metering = build_metering(&self.spec);
    }

    pub fn set_luminaire_format(&mut self, format: Option<LuminaireFormat>) {
        self.spec.luminaire_format = format;
        self.spec
            .device_types
            .retain(|t| *t != DEVICE_TYPE_LUMINAIRE_INFO);
        if format.is_some() {
            self.spec.device_types.push(DEVICE_TYPE_LUMINAIRE_INFO);
        }
        self.luminaire = build_luminaire(&self.spec);
        self.bank0 = build_bank0(&self.spec);
    }

    pub fn set_bus_unit_extension(&mut self, enabled: bool) {
        self.spec.bus_unit_extension = enabled;
        self.bank0 = build_bank0(&self.spec);
    }

    pub fn memory_location(&self, bank: u8, offset: u8) -> Option<u8> {
        match bank {
            MEMORY_BANK_IDENTITY => self.identity_location(offset),
            MEMORY_BANK_PROFILE => self.profile_location(offset),
            _ if self.metering.is_some() => {
                self.metering.as_ref().and_then(|m| m.location(bank, offset))
            }
            _ => None,
        }
    }

    pub fn implements_bank(&self, bank: u8) -> bool {
        match bank {
            MEMORY_BANK_IDENTITY | MEMORY_BANK_PROFILE => true,
            _ => self
                .metering
                .as_ref()
                .is_some_and(|m| m.implements_bank(bank)),
        }
    }

    pub fn matches(&self, address: DaliAddress) -> bool {
        if !self.enabled {
            return false;
        }
        match address {
            DaliAddress::Short(short) => self.spec.short_address == Some(short),
            DaliAddress::Group(group) => self.groups & (1u16 << group) != 0,
            DaliAddress::Broadcast => true,
            DaliAddress::BroadcastUnaddressed => self.spec.short_address.is_none(),
        }
    }

    pub fn is_dt8(&self) -> bool {
        self.spec.device_types.contains(&DeviceType::Color.code())
    }

    pub fn is_dt6(&self) -> bool {
        self.spec.device_types.contains(&DeviceType::Led.code())
    }

    pub(crate) fn holds(&self, frame: u16) -> bool {
        self.pending_repeat == Some(frame)
    }

    pub(crate) fn hear(&mut self, frame: u16, repeats: bool) -> Heard {
        if self.pending_repeat == Some(frame) {
            self.pending_repeat = None;
            return Heard {
                executes: true,
                completed_pair: true,
                split: false,
            };
        }
        let split = self.pending_repeat.take().is_some();
        self.pending_repeat = repeats.then_some(frame);
        Heard {
            executes: !repeats,
            completed_pair: false,
            split,
        }
    }

    pub fn automatic_activation(&self) -> bool {
        gear_features_automatic_activation(self.gear_features)
    }

    // IEC 62386-209 §9.12.5, Table 5
    pub(crate) fn arc_power_activation(&mut self) {
        if self.automatic_activation() {
            self.activate_pending_color();
        }
    }

    pub fn activate_pending_color(&mut self) {
        match self.pending_mode {
            ColorMode::None => {}
            ColorMode::Xy => {
                self.color_mode = ColorMode::Xy;
                self.color_x = self.pending_x;
                self.color_y = self.pending_y;
                self.unlink_all_channels();
            }
            ColorMode::Cct => {
                self.commit_pending_cct();
                self.unlink_all_channels();
            }
            ColorMode::Rgb => self.commit_pending_rgbwaf(),
        }
        self.pending_mode = ColorMode::None;
        self.rgbwaf.pending_control = RGBWAF_CONTROL_MASK;
    }

    // IEC 62386-209 §9.12.5
    fn commit_pending_rgbwaf(&mut self) {
        self.color_mode = ColorMode::Rgb;
        if self.rgbwaf.pending_control != RGBWAF_CONTROL_MASK {
            self.rgbwaf.control = self.rgbwaf.pending_control;
        }
        self.rgbwaf.levels = self.rgbwaf.pending_levels;
    }

    // IEC 62386-209 §11.3.4.1
    fn unlink_all_channels(&mut self) {
        self.rgbwaf.control &= !RGBWAF_CONTROL_ALL_CHANNELS;
    }

    // IEC 62386-209 §9.13
    fn commit_pending_cct(&mut self) {
        self.color_mode = ColorMode::Cct;
        let clamped = self
            .pending_ct
            .clamp(self.effective_coolest(), self.effective_warmest());
        self.tc_out_of_range = clamped != self.pending_ct;
        self.color_ct = clamped;
    }

    pub(crate) fn tc_step(&mut self, warmer: bool) {
        if self.color_mode != ColorMode::Cct {
            return;
        }
        let stepped = if warmer {
            self.color_ct.saturating_add(1).min(self.effective_warmest())
        } else {
            self.color_ct.saturating_sub(1).max(self.effective_coolest())
        };
        self.color_ct = stepped;
        self.tc_out_of_range = false;
    }

    pub(crate) fn xy_step(&mut self, y_axis: bool, up: bool) {
        if self.color_mode != ColorMode::Xy {
            return;
        }
        let value = if y_axis { &mut self.color_y } else { &mut self.color_x };
        *value = if up {
            value.saturating_add(XY_STEP).min(XY_COORDINATE_MAX)
        } else {
            value.saturating_sub(XY_STEP)
        };
    }

    // IEC 62386-209 §9.13, Table 7
    pub(crate) fn store_tc_limit(&mut self, value: u16, selector: u8) {
        match selector {
            TC_LIMIT_SELECTOR_COOLEST => self.store_rendered_tc_limit(false, value),
            TC_LIMIT_SELECTOR_WARMEST => self.store_rendered_tc_limit(true, value),
            TC_LIMIT_SELECTOR_PHYSICAL_COOLEST => self.store_physical_tc_limit(false, value),
            TC_LIMIT_SELECTOR_PHYSICAL_WARMEST => self.store_physical_tc_limit(true, value),
            _ => return,
        }
        if self.color_mode == ColorMode::Cct {
            let snapped = self
                .color_ct
                .clamp(self.effective_coolest(), self.effective_warmest());
            if snapped != self.color_ct {
                self.color_ct = snapped;
                self.tc_out_of_range = true;
            }
        }
    }

    fn store_rendered_tc_limit(&mut self, warmest: bool, value: u16) {
        if value == COLOUR_VALUE_MASK {
            *self.rendered_limit_mut(warmest) = COLOUR_VALUE_MASK;
            return;
        }
        let clamped = value.clamp(self.spec.tc_coolest_mirek, self.spec.tc_warmest_mirek);
        *self.rendered_limit_mut(warmest) = clamped;
        if warmest && clamped < self.effective_coolest() {
            self.tc_coolest_limit = clamped;
        }
        if !warmest && clamped > self.effective_warmest() {
            self.tc_warmest_limit = clamped;
        }
    }

    fn store_physical_tc_limit(&mut self, warmest: bool, value: u16) {
        if value == COLOUR_VALUE_MASK {
            return;
        }
        if warmest {
            self.spec.tc_warmest_mirek = value.max(self.spec.tc_coolest_mirek);
        } else {
            self.spec.tc_coolest_mirek = value.min(self.spec.tc_warmest_mirek);
        }
        let (floor, ceil) = (self.spec.tc_coolest_mirek, self.spec.tc_warmest_mirek);
        for warm in [false, true] {
            let effective = if warm {
                self.effective_warmest()
            } else {
                self.effective_coolest()
            };
            let bounded = effective.clamp(floor, ceil);
            if bounded != effective {
                *self.rendered_limit_mut(warm) = bounded;
            }
        }
    }

    fn rendered_limit_mut(&mut self, warmest: bool) -> &mut u16 {
        if warmest {
            &mut self.tc_warmest_limit
        } else {
            &mut self.tc_coolest_limit
        }
    }

    fn effective_coolest(&self) -> u16 {
        if self.tc_coolest_limit == COLOUR_VALUE_MASK {
            self.spec.tc_coolest_mirek
        } else {
            self.tc_coolest_limit
        }
    }

    fn effective_warmest(&self) -> u16 {
        if self.tc_warmest_limit == COLOUR_VALUE_MASK {
            self.spec.tc_warmest_mirek
        } else {
            self.tc_warmest_limit
        }
    }

    // IEC 62386-209 §9.12.6
    pub(crate) fn load_report_from_actual(&mut self) {
        self.report = match self.color_mode {
            ColorMode::None => ColourSetting::mask(),
            ColorMode::Xy => ColourSetting {
                colour_type: COLOUR_TYPE_BYTE_XY,
                x: self.color_x,
                y: self.color_y,
                ..ColourSetting::mask()
            },
            ColorMode::Cct => ColourSetting {
                colour_type: COLOUR_TYPE_BYTE_TC,
                tc: self.color_ct,
                ..ColourSetting::mask()
            },
            ColorMode::Rgb => ColourSetting {
                colour_type: COLOUR_TYPE_BYTE_RGBWAF,
                levels: self.rgbwaf.levels,
                control: self.rgbwaf.control,
                ..ColourSetting::mask()
            },
        };
    }

    pub(crate) fn load_report_from_scene(&mut self, scene: u8) {
        self.report = self
            .scene_colours
            .get(usize::from(scene))
            .copied()
            .unwrap_or_else(ColourSetting::mask);
    }

    pub(crate) fn load_report_mask(&mut self) {
        self.report = ColourSetting::mask();
    }

    pub(crate) fn copy_report_to_temporary(&mut self) {
        let report = self.report;
        self.load_temporaries_from(&report);
    }

    fn load_temporaries_from(&mut self, setting: &ColourSetting) {
        match setting.colour_type {
            COLOUR_TYPE_BYTE_XY => {
                self.pending_mode = ColorMode::Xy;
                self.pending_x = setting.x;
                self.pending_y = setting.y;
                self.rgbwaf.pending_control = RGBWAF_CONTROL_MASK;
            }
            COLOUR_TYPE_BYTE_TC => {
                self.pending_mode = ColorMode::Cct;
                self.pending_ct = setting.tc;
                self.rgbwaf.pending_control = RGBWAF_CONTROL_MASK;
            }
            COLOUR_TYPE_BYTE_RGBWAF => {
                self.pending_mode = ColorMode::Rgb;
                self.rgbwaf.pending_levels = setting.levels;
                self.rgbwaf.pending_control = setting.control;
            }
            _ => {
                self.pending_mode = ColorMode::None;
                self.rgbwaf.pending_control = RGBWAF_CONTROL_MASK;
            }
        }
    }

    pub(crate) fn store_scene_colour(&mut self, scene: u8) {
        let staged = match self.pending_mode {
            ColorMode::None => return,
            ColorMode::Xy => ColourSetting {
                colour_type: COLOUR_TYPE_BYTE_XY,
                x: self.pending_x,
                y: self.pending_y,
                ..ColourSetting::mask()
            },
            ColorMode::Cct => ColourSetting {
                colour_type: COLOUR_TYPE_BYTE_TC,
                tc: self.pending_ct,
                ..ColourSetting::mask()
            },
            ColorMode::Rgb => ColourSetting {
                colour_type: COLOUR_TYPE_BYTE_RGBWAF,
                levels: self.rgbwaf.pending_levels,
                control: self.rgbwaf.pending_control,
                ..ColourSetting::mask()
            },
        };
        if let Some(slot) = self.scene_colours.get_mut(usize::from(scene)) {
            *slot = staged;
            self.pending_mode = ColorMode::None;
            self.rgbwaf.pending_control = RGBWAF_CONTROL_MASK;
        }
    }

    pub fn set_level(&mut self, level: u8) {
        self.level = match level {
            0 => 0,
            l => l.clamp(self.min_level, self.max_level),
        };
        // IEC 62386-102 §9.16.5
        self.limit_error = level != 0 && self.level != level;
        if self.level > 0 {
            self.last_active_level = self.level;
        }
    }

    pub(crate) fn cancel_device_type_walk(&mut self) {
        self.device_type_walk = None;
    }

    fn set_min_level(&mut self, dtr0: u8) {
        self.min_level = if dtr0 <= self.spec.phm {
            self.spec.phm
        } else if dtr0 >= self.max_level {
            self.max_level
        } else {
            dtr0
        };
        self.readjust_level_into_bounds();
    }

    fn set_max_level(&mut self, dtr0: u8) {
        self.max_level = if self.min_level >= dtr0 {
            self.min_level
        } else if dtr0 == DAPC_MASK_LEVEL {
            0xFE
        } else {
            dtr0
        };
        self.readjust_level_into_bounds();
    }

    fn readjust_level_into_bounds(&mut self) {
        if self.level == 0 {
            return;
        }
        let clamped = self.level.clamp(self.min_level, self.max_level);
        if clamped != self.level {
            self.level = clamped;
            self.last_active_level = clamped;
            self.limit_error = true;
        }
    }

    pub fn apply_dapc(&mut self, level: u8) {
        // IEC 62386-102 §9.16.9
        self.power_cycle_seen = false;
        if level != DAPC_MASK_LEVEL {
            self.set_level(level);
        }
    }

    // IEC 62386-102 §9.16.4
    pub fn lamp_on(&self) -> bool {
        self.level > 0 && !self.lamp_starting && !self.lamp_failure && !self.thermal_shut_down()
    }

    pub fn failure_bits(&self) -> FailureStatusBits {
        decode_failure_status(self.faults.failure_status)
    }

    pub fn thermal_shut_down(&self) -> bool {
        self.is_dt6() && self.failure_bits().thermal_shut_down
    }

    // IEC 62386-207 §11.3.4.2
    pub fn lamp_failure_from_207(&self) -> bool {
        if !self.is_dt6() {
            return false;
        }
        let b = self.failure_bits();
        b.short_circuit
            || b.open_circuit
            || b.load_decrease
            || b.load_increase
            || b.current_protector_active
    }

    pub fn status_byte(&self) -> u8 {
        let mut raw = self.faults.forced_status;
        if self.lamp_failure || self.lamp_failure_from_207() {
            raw |= STATUS_LAMP_FAILURE;
        }
        if self.lamp_on() {
            raw |= STATUS_LAMP_ON;
        }
        if self.limit_error {
            raw |= STATUS_LIMIT_ERROR;
        }
        if self.reset_state {
            raw |= STATUS_RESET_STATE;
        }
        if self.power_cycle_seen {
            raw |= STATUS_POWER_CYCLE_SEEN;
        }
        if self.spec.short_address.is_none() {
            raw |= STATUS_MISSING_SHORT_ADDRESS;
        }
        raw
    }

    pub fn dt8_colour_status(&self) -> u8 {
        let active = match self.color_mode {
            ColorMode::None => 0,
            ColorMode::Xy => DT8_STATUS_XY_ACTIVE,
            ColorMode::Cct => DT8_STATUS_TC_ACTIVE,
            ColorMode::Rgb => DT8_STATUS_RGB_ACTIVE,
        };
        if self.tc_out_of_range && matches!(self.color_mode, ColorMode::Cct) {
            active | COLOUR_STATUS_TC_OUT_OF_RANGE
        } else {
            active
        }
    }

    // IEC 62386-209 Table 11
    pub fn color_value(&self, value_id: u8) -> Option<u16> {
        match value_id {
            0..=15 => self.active_colour_value(value_id),
            64..=131 => self.primary_and_limit_value(value_id),
            192..=208 => self.temporary_colour_value(value_id),
            224..=240 => self.report_colour_value(value_id),
            _ => None,
        }
    }

    fn active_colour_value(&self, value_id: u8) -> Option<u16> {
        let (xy, tc, rgb) = (self.spec.xy_capable, self.spec.tc_capable, self.spec.rgb_capable());
        match value_id {
            COLOUR_VALUE_X if xy => Some(self.active_or_mask(ColorMode::Xy, self.color_x)),
            COLOUR_VALUE_Y if xy => Some(self.active_or_mask(ColorMode::Xy, self.color_y)),
            COLOUR_VALUE_TC if tc => Some(self.active_or_mask(ColorMode::Cct, self.color_ct)),
            COLOUR_VALUE_RED..=COLOUR_VALUE_FREECOLOUR if rgb => {
                if self.color_mode == ColorMode::Rgb {
                    self.channel_dim_level(value_id)
                } else {
                    Some(u16::from(COLOUR_VALUE_LEVEL_MASK))
                }
            }
            COLOUR_VALUE_RGBWAF_CONTROL if rgb => Some(if self.color_mode == ColorMode::Rgb {
                u16::from(self.rgbwaf.control)
            } else {
                u16::from(RGBWAF_CONTROL_MASK)
            }),
            _ => None,
        }
    }

    fn primary_and_limit_value(&self, value_id: u8) -> Option<u16> {
        let tc = self.spec.tc_capable;
        match value_id {
            64..=81 => Some(COLOUR_VALUE_MASK),
            COLOUR_VALUE_NUMBER_OF_PRIMARIES => Some(0),
            COLOUR_VALUE_TC_COOLEST if tc => Some(self.tc_coolest_limit),
            COLOUR_VALUE_TC_PHYSICAL_COOLEST if tc => Some(self.spec.tc_coolest_mirek),
            COLOUR_VALUE_TC_WARMEST if tc => Some(self.tc_warmest_limit),
            COLOUR_VALUE_TC_PHYSICAL_WARMEST if tc => Some(self.spec.tc_warmest_mirek),
            _ => None,
        }
    }

    fn temporary_colour_value(&self, value_id: u8) -> Option<u16> {
        let (xy, tc, rgb) = (self.spec.xy_capable, self.spec.tc_capable, self.spec.rgb_capable());
        match value_id {
            COLOUR_VALUE_TEMPORARY_X if xy => {
                Some(self.staged_or_mask(ColorMode::Xy, self.pending_x))
            }
            COLOUR_VALUE_TEMPORARY_Y if xy => {
                Some(self.staged_or_mask(ColorMode::Xy, self.pending_y))
            }
            COLOUR_VALUE_TEMPORARY_TC if tc => {
                Some(self.staged_or_mask(ColorMode::Cct, self.pending_ct))
            }
            201..=206 if rgb => Some(if self.pending_mode == ColorMode::Rgb {
                self.staged_channel_level(value_id - 201)
            } else {
                u16::from(COLOUR_VALUE_LEVEL_MASK)
            }),
            COLOUR_VALUE_TEMPORARY_RGBWAF_CONTROL if rgb => {
                Some(if self.pending_mode == ColorMode::Rgb {
                    u16::from(self.rgbwaf.pending_control)
                } else {
                    u16::from(RGBWAF_CONTROL_MASK)
                })
            }
            COLOUR_VALUE_TEMPORARY_COLOUR_TYPE => {
                Some(u16::from(colour_type_byte(self.pending_mode)))
            }
            _ => None,
        }
    }

    fn report_colour_value(&self, value_id: u8) -> Option<u16> {
        let (xy, tc, rgb) = (self.spec.xy_capable, self.spec.tc_capable, self.spec.rgb_capable());
        match value_id {
            COLOUR_VALUE_REPORT_X if xy => {
                Some(self.report_or_mask(COLOUR_TYPE_BYTE_XY, self.report.x))
            }
            COLOUR_VALUE_REPORT_Y if xy => {
                Some(self.report_or_mask(COLOUR_TYPE_BYTE_XY, self.report.y))
            }
            COLOUR_VALUE_REPORT_TC if tc => {
                Some(self.report_or_mask(COLOUR_TYPE_BYTE_TC, self.report.tc))
            }
            COLOUR_VALUE_REPORT_RED..=COLOUR_VALUE_REPORT_FREECOLOUR if rgb => {
                Some(if self.report.colour_type == COLOUR_TYPE_BYTE_RGBWAF {
                    self.report_channel_level(value_id - COLOUR_VALUE_REPORT_RED)
                } else {
                    u16::from(COLOUR_VALUE_LEVEL_MASK)
                })
            }
            COLOUR_VALUE_REPORT_RGBWAF_CONTROL if rgb => {
                Some(if self.report.colour_type == COLOUR_TYPE_BYTE_RGBWAF {
                    u16::from(self.report.control)
                } else {
                    u16::from(RGBWAF_CONTROL_MASK)
                })
            }
            COLOUR_VALUE_REPORT_COLOUR_TYPE => Some(u16::from(self.report.colour_type)),
            _ => None,
        }
    }

    fn active_or_mask(&self, mode: ColorMode, value: u16) -> u16 {
        if self.color_mode == mode {
            value
        } else {
            COLOUR_VALUE_MASK
        }
    }

    fn staged_or_mask(&self, mode: ColorMode, value: u16) -> u16 {
        if self.pending_mode == mode {
            value
        } else {
            COLOUR_VALUE_MASK
        }
    }

    fn report_or_mask(&self, colour_type: u8, value: u16) -> u16 {
        if self.report.colour_type == colour_type {
            value
        } else {
            COLOUR_VALUE_MASK
        }
    }

    fn staged_channel_level(&self, channel: u8) -> u16 {
        if channel >= self.spec.rgbwaf_channels {
            return u16::from(COLOUR_VALUE_LEVEL_MASK);
        }
        u16::from(self.rgbwaf.pending_levels[usize::from(channel)])
    }

    fn report_channel_level(&self, channel: u8) -> u16 {
        if channel >= self.spec.rgbwaf_channels {
            return u16::from(COLOUR_VALUE_LEVEL_MASK);
        }
        u16::from(self.report.levels[usize::from(channel)])
    }

    fn channel_dim_level(&self, value_id: u8) -> Option<u16> {
        let channel = usize::from(value_id - COLOUR_VALUE_RED);
        if channel >= usize::from(self.spec.rgbwaf_channels) {
            return Some(u16::from(COLOUR_VALUE_LEVEL_MASK));
        }
        Some(u16::from(self.rgbwaf.levels[channel]))
    }

    // IEC 62386-209 §11.3.2
    pub fn actual_level_answer(&self) -> u8 {
        let thermal = self.failure_bits();
        if self.is_dt6() && (thermal.thermal_shut_down || thermal.thermal_overload) {
            return DAPC_MASK_LEVEL;
        }
        if self.color_mode != ColorMode::Rgb {
            return self.level;
        }
        let linked = (self.rgbwaf.control & RGBWAF_CONTROL_ALL_CHANNELS).count_ones();
        let normalised =
            self.rgbwaf.control & RGBWAF_CONTROL_TYPE_MASK == RGBWAF_CONTROL_NORMALISED;
        if linked == 1 || normalised {
            self.level
        } else {
            DAPC_MASK_LEVEL
        }
    }

    pub fn rgbwaf_control_byte(&self) -> Option<u8> {
        (self.spec.rgbwaf_channels > 0).then_some(self.rgbwaf.control)
    }

    pub fn gear_features_byte(&self) -> u8 {
        self.gear_features
    }

    pub fn dt8_features(&self) -> u8 {
        let mut features = 0u8;
        if self.spec.xy_capable {
            features |= DT8_FEATURE_XY;
        }
        if self.spec.tc_capable {
            features |= DT8_FEATURE_TC;
        }
        features |= (self.spec.rgbwaf_channels & 0x07) << DT8_FEATURE_RGBWAF_CHANNEL_SHIFT;
        features
    }
}

pub fn apply_standard_write(gear: &mut Gear, command: StandardCommand, dtr0: u8) {
    gear.device_type_walk = None;
    apply_identify_state(gear, command);
    apply_level_write(gear, command);
    apply_config_write(gear, command, dtr0);
}

// IEC 62386-102 §9.14.3.1
fn apply_identify_state(gear: &mut Gear, command: StandardCommand) {
    gear.identifying = match command {
        StandardCommand::IdentifyDevice => true,
        StandardCommand::RecallMinLevel | StandardCommand::RecallMaxLevel => gear.identifying,
        _ => false,
    };
}

// IEC 62386-209 §11.1.2
fn is_arc_power_change(command: StandardCommand) -> bool {
    matches!(
        command,
        StandardCommand::DirectArcPower { .. }
            | StandardCommand::Off
            | StandardCommand::RecallMaxLevel
            | StandardCommand::RecallMinLevel
            | StandardCommand::GoToLastActiveLevel
            | StandardCommand::Up
            | StandardCommand::Down
            | StandardCommand::StepUp
            | StandardCommand::StepDown
            | StandardCommand::StepDownAndOff
            | StandardCommand::OnAndStepUp
    )
}

// IEC 62386-102 §9.16.9
fn clears_power_cycle_seen(command: StandardCommand) -> bool {
    is_arc_power_change(command) || matches!(command, StandardCommand::GoToScene { .. })
}

fn apply_level_write(gear: &mut Gear, command: StandardCommand) {
    if clears_power_cycle_seen(command) {
        gear.power_cycle_seen = false;
    }
    if is_arc_power_change(command) {
        gear.arc_power_activation();
    }
    match command {
        StandardCommand::DirectArcPower { level } => gear.apply_dapc(level),
        StandardCommand::Off => gear.set_level(0),
        StandardCommand::RecallMaxLevel => gear.set_level(gear.max_level),
        StandardCommand::RecallMinLevel => gear.set_level(gear.min_level),
        StandardCommand::GoToLastActiveLevel => gear.set_level(gear.last_active_level),
        StandardCommand::Up => level_up(gear, UP_DOWN_LEVEL_STEP),
        StandardCommand::Down => level_down(gear, UP_DOWN_LEVEL_STEP),
        StandardCommand::StepUp => level_up(gear, 1),
        StandardCommand::StepDown => level_down(gear, 1),
        StandardCommand::StepDownAndOff => step_down_and_off(gear),
        StandardCommand::OnAndStepUp => on_and_step_up(gear),
        StandardCommand::GoToScene { scene } => recall_scene(gear, scene),
        _ => {}
    }
}

// IEC 62386-102 §11.3.3, §11.3.5
fn level_up(gear: &mut Gear, delta: u8) {
    if gear.level > 0 {
        gear.set_level(gear.level.saturating_add(delta));
    }
}

fn level_down(gear: &mut Gear, delta: u8) {
    if gear.level > 0 {
        gear.set_level(gear.level.saturating_sub(delta).max(gear.min_level));
    }
}

fn step_down_and_off(gear: &mut Gear) {
    if gear.level <= gear.min_level {
        gear.set_level(0);
    } else {
        gear.set_level(gear.level.saturating_sub(1));
    }
}

fn on_and_step_up(gear: &mut Gear) {
    if gear.level == 0 {
        gear.set_level(gear.min_level);
    } else {
        gear.set_level(gear.level.saturating_add(1));
    }
}

// IEC 62386-209 §9.11.4, Table 6
fn recall_scene(gear: &mut Gear, scene: u8) {
    let colour = gear
        .scene_colours
        .get(usize::from(scene))
        .copied()
        .unwrap_or_else(ColourSetting::mask);
    if colour.colour_type != COLOUR_TYPE_BYTE_MASK {
        gear.load_temporaries_from(&colour);
        if gear.automatic_activation() {
            gear.activate_pending_color();
        }
    }
    if let Some(&level) = gear.scenes.get(usize::from(scene)) {
        if level != SCENE_UNSET {
            gear.set_level(level);
        }
    }
}

fn apply_config_write(gear: &mut Gear, command: StandardCommand, dtr0: u8) {
    match command {
        StandardCommand::SetMaxLevel => gear.set_max_level(dtr0),
        StandardCommand::SetMinLevel => gear.set_min_level(dtr0),
        StandardCommand::SetPowerOnLevel => gear.power_on_level = dtr0,
        StandardCommand::SetSystemFailureLevel => gear.system_failure_level = dtr0,
        StandardCommand::SetFadeTime => gear.fade_time = dtr0,
        StandardCommand::SetFadeRate => gear.fade_rate = dtr0,
        StandardCommand::SetExtendedFadeTime => gear.extended_fade_time = dtr0,
        StandardCommand::SetScene { scene } => {
            set_scene_slot(gear, scene, dtr0);
            gear.store_scene_colour(scene);
        }
        StandardCommand::RemoveScene { scene } => {
            set_scene_slot(gear, scene, SCENE_UNSET);
            if let Some(slot) = gear.scene_colours.get_mut(usize::from(scene)) {
                *slot = ColourSetting::mask();
            }
        }
        StandardCommand::AddToGroup { group } => set_group_bit(gear, group, true),
        StandardCommand::RemoveFromGroup { group } => set_group_bit(gear, group, false),
        StandardCommand::Reset => reset_gear(gear),
        _ => return,
    }
    if !matches!(command, StandardCommand::Reset) {
        gear.reset_state = false;
    }
}

fn set_scene_slot(gear: &mut Gear, scene: u8, level: u8) {
    if let Some(slot) = gear.scenes.get_mut(usize::from(scene)) {
        *slot = level;
    }
}

fn set_group_bit(gear: &mut Gear, group: u8, member: bool) {
    if group < GROUP_COUNT {
        let bit = 1u16 << group;
        if member {
            gear.groups |= bit;
        } else {
            gear.groups &= !bit;
        }
    }
}

// IEC 62386-209 Table 8
fn reset_gear(gear: &mut Gear) {
    let spec = gear.spec.clone();
    let enabled = gear.enabled;
    let faults = gear.faults;
    let executes_now = gear.executes_now;
    let color_mode = gear.color_mode;
    let (color_x, color_y, color_ct) = (gear.color_x, gear.color_y, gear.color_ct);
    let tc_out_of_range = gear.tc_out_of_range;
    let rgbwaf_levels = gear.rgbwaf.levels;
    let rgbwaf_control = gear.rgbwaf.control;
    *gear = Gear::new(spec);
    gear.enabled = enabled;
    gear.faults = faults;
    gear.executes_now = executes_now;
    gear.power_cycle_seen = false;
    gear.color_mode = color_mode;
    gear.color_x = color_x;
    gear.color_y = color_y;
    gear.color_ct = color_ct;
    gear.tc_out_of_range = tc_out_of_range;
    gear.rgbwaf.levels = rgbwaf_levels;
    gear.rgbwaf.control = rgbwaf_control;
}

pub fn gear_query_reply(gear: &mut Gear, command: StandardCommand) -> TransferOutcome {
    if !matches!(
        command,
        StandardCommand::QueryDeviceType | StandardCommand::QueryNextDeviceType
    ) {
        gear.device_type_walk = None;
    }
    if gear.is_dt8() {
        match command {
            StandardCommand::QueryActualLevel => gear.load_report_from_actual(),
            StandardCommand::QuerySceneLevel { scene } => gear.load_report_from_scene(scene),
            StandardCommand::QueryPowerOnLevel | StandardCommand::QuerySystemFailureLevel => {
                gear.load_report_mask()
            }
            _ => {}
        }
    }
    if let Some(outcome) = level_query_reply(gear, command) {
        return outcome;
    }
    if let Some(outcome) = identity_query_reply(gear, command) {
        return outcome;
    }
    TransferOutcome::NoAnswer
}

fn answer_if(condition: bool, value: u8) -> TransferOutcome {
    if condition {
        TransferOutcome::Answer(value)
    } else {
        TransferOutcome::NoAnswer
    }
}

fn level_query_reply(gear: &Gear, command: StandardCommand) -> Option<TransferOutcome> {
    let answer = |v: u8| Some(TransferOutcome::Answer(v));
    match command {
        StandardCommand::QueryStatus => answer(gear.status_byte()),
        StandardCommand::QueryActualLevel => answer(gear.actual_level_answer()),
        StandardCommand::QueryMaxLevel => answer(gear.max_level),
        StandardCommand::QueryMinLevel => answer(gear.min_level),
        StandardCommand::QueryPowerOnLevel => answer(gear.power_on_level),
        StandardCommand::QuerySystemFailureLevel => answer(gear.system_failure_level),
        StandardCommand::QueryPhysicalMinimum => answer(gear.spec.phm),
        StandardCommand::QueryFadeTimeFadeRate => {
            answer((gear.fade_time << 4) | (gear.fade_rate & 0x0F))
        }
        StandardCommand::QueryExtendedFadeTime => answer(gear.extended_fade_time),
        StandardCommand::QueryControlGearPresent => answer(DALI_YES),
        StandardCommand::QueryLampFailure => Some(answer_if(gear.lamp_failure, DALI_YES)),
        StandardCommand::QueryLampPowerOn => Some(answer_if(gear.lamp_on(), DALI_YES)),
        StandardCommand::QueryLimitError => Some(answer_if(gear.limit_error, DALI_YES)),
        StandardCommand::QueryOperatingMode => answer(OPERATING_MODE_STANDARD),
        StandardCommand::QueryResetState => Some(answer_if(gear.reset_state, DALI_YES)),
        StandardCommand::QueryControlGearFailure => Some(answer_if(
            gear.status_byte() & STATUS_GEAR_FAILURE != 0,
            DALI_YES,
        )),
        StandardCommand::QueryManufacturerSpecificMode => Some(answer_if(
            is_manufacturer_specific_mode(OPERATING_MODE_STANDARD),
            DALI_YES,
        )),
        _ => None,
    }
}

fn identity_query_reply(gear: &mut Gear, command: StandardCommand) -> Option<TransferOutcome> {
    let answer = |v: u8| Some(TransferOutcome::Answer(v));
    match command {
        StandardCommand::QueryDeviceType => Some(device_type_reply(gear)),
        StandardCommand::QueryNextDeviceType => Some(next_device_type_reply(gear)),
        StandardCommand::QueryGroups0To7 => answer(gear.groups as u8),
        StandardCommand::QueryGroups8To15 => answer((gear.groups >> 8) as u8),
        StandardCommand::QuerySceneLevel { scene } => {
            answer(*gear.scenes.get(usize::from(scene)).unwrap_or(&SCENE_UNSET))
        }
        StandardCommand::QueryRandomAddressH => answer((gear.spec.random_address >> 16) as u8),
        StandardCommand::QueryRandomAddressM => answer((gear.spec.random_address >> 8) as u8),
        StandardCommand::QueryRandomAddressL => answer(gear.spec.random_address as u8),
        StandardCommand::QueryVersionNumber => answer(VERSION_NUMBER_102),
        StandardCommand::QueryLightSourceType => answer(LIGHT_SOURCE_LED),
        StandardCommand::QueryMissingShortAddress => {
            Some(answer_if(gear.spec.short_address.is_none(), DALI_YES))
        }
        _ => None,
    }
}

fn device_type_reply(gear: &mut Gear) -> TransferOutcome {
    match gear.spec.device_types.len() {
        0 => {
            gear.device_type_walk = None;
            TransferOutcome::Answer(DEVICE_TYPE_LIST_END)
        }
        1 => {
            gear.device_type_walk = None;
            TransferOutcome::Answer(gear.spec.device_types[0])
        }
        _ => {
            gear.device_type_walk = Some(0);
            TransferOutcome::Answer(DEVICE_TYPE_MULTIPLE)
        }
    }
}

fn next_device_type_reply(gear: &mut Gear) -> TransferOutcome {
    // IEC 62386-102 §11.5.13
    let Some(cursor) = gear.device_type_walk else {
        return TransferOutcome::NoAnswer;
    };
    match gear.spec.device_types.get(cursor) {
        Some(&device_type) => {
            gear.device_type_walk = Some(cursor + 1);
            TransferOutcome::Answer(device_type)
        }
        None => TransferOutcome::Answer(DEVICE_TYPE_LIST_END),
    }
}
