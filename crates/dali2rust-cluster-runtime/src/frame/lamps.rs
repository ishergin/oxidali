use dali2rust_contracts::msg::{ColorMode, DeviceType, PowerState, RuntimeSource};
use dali2rust_domain::dali::device::MAX_ARC_POWER_LEVEL;
use serde::{Deserialize, Serialize};

use super::codec::FrameKind;
use super::command::{is_record_ref, RecordRef};
use super::limits::{is_adapter, is_group, is_lamp, MAX_INSTALLATION_BYTES, MAX_NAME_BYTES};
use super::page::{InRange, Page, PageElements, PageKind, PageWriter};

pub const MAX_DISCOVERY_PREFIX_BYTES: usize = 32;

const MEMBERS_PER_GROUP: u8 = dali2rust_domain::registry::VIRTUAL_LAMP_COUNT;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterScope {
    pub adapter: u8,
}

impl InRange for AdapterScope {
    fn in_range(&self) -> bool {
        is_adapter(self.adapter)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LampColour {
    pub mode: ColorMode,
    pub kelvin: Option<u16>,
    pub xy: Option<[u16; 2]>,
    pub rgb: Option<[u8; 3]>,
    pub waf: Option<[u8; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LampState {
    pub lamp: u8,
    pub power: PowerState,
    pub level: Option<u8>,
    pub colour: LampColour,
    pub age_ms: Option<u32>,
    pub source: Option<RuntimeSource>,
    pub absent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartState {
    pub group: u8,
    pub channel: RecordRef,
    pub commanded: bool,
    pub lit: u8,
    pub known: u8,
    pub members: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateElement {
    Lamp(LampState),
    Part(PartState),
}

impl InRange for StateElement {
    fn in_range(&self) -> bool {
        match self {
            Self::Lamp(lamp) => is_lamp(lamp.lamp) && lamp.level.is_none_or(|level| level <= MAX_ARC_POWER_LEVEL),
            Self::Part(part) => {
                let counts = part.lit <= part.known && part.known <= part.members && part.members <= MEMBERS_PER_GROUP;
                is_group(part.group) && is_record_ref(&part.channel) && counts
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LampCapabilities {
    pub brightness: bool,
    pub cct: bool,
    pub xy: bool,
    pub rgb: bool,
    pub rgbwaf: bool,
    pub scenes: bool,
    pub groups: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KelvinRange {
    pub min: u16,
    pub max: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LampInfo<'a> {
    pub lamp: u8,
    pub name: &'a str,
    pub device_type: DeviceType,
    pub device_type_overridden: bool,
    pub device_types: u64,
    pub color_mode: ColorMode,
    pub color_mode_overridden: bool,
    pub capabilities: LampCapabilities,
    pub kelvin_range: Option<KelvinRange>,
    pub bound: bool,
    pub ha_entity: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HaIdentity<'a> {
    pub controller_id: &'a str,
    pub discovery_prefix: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LampInfoElement<'a> {
    Owner(#[serde(borrow)] HaIdentity<'a>),
    Lamp(#[serde(borrow)] LampInfo<'a>),
}

impl InRange for LampInfoElement<'_> {
    fn in_range(&self) -> bool {
        match self {
            Self::Owner(owner) => {
                owner.controller_id.len() <= MAX_INSTALLATION_BYTES
                    && owner.discovery_prefix.len() <= MAX_DISCOVERY_PREFIX_BYTES
            }
            Self::Lamp(lamp) => {
                let range = lamp.kelvin_range.is_none_or(|range| range.min <= range.max);
                is_lamp(lamp.lamp) && lamp.name.len() <= MAX_NAME_BYTES && range
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LampStateKind;

impl PageKind for LampStateKind {
    const KIND: FrameKind = FrameKind::LampState;
    type Scope = AdapterScope;
    type Element<'a> = StateElement;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LampInfoKind;

impl PageKind for LampInfoKind {
    const KIND: FrameKind = FrameKind::LampInfo;
    type Scope = AdapterScope;
    type Element<'a> = LampInfoElement<'a>;
}

pub type LampStatePage<'a> = Page<'a, LampStateKind>;

pub type LampInfoPage<'a> = Page<'a, LampInfoKind>;

pub type StateElements<'a> = PageElements<'a, LampStateKind>;

pub type LampInfoElements<'a> = PageElements<'a, LampInfoKind>;

pub type LampStateWriter<'b> = PageWriter<'b, LampStateKind>;

pub type LampInfoWriter<'b> = PageWriter<'b, LampInfoKind>;
