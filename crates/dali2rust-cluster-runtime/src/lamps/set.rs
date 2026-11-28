use core::ptr;

use dali2rust_contracts::msg::{fixed_text_32, fixed_text_64, ColorMode, DeviceType, FixedText32, FixedText64};
use dali2rust_domain::registry::{GROUP_COUNT, VIRTUAL_LAMP_COUNT};

use crate::frame::{
    HaIdentity, KelvinRange, LampCapabilities, LampInfo, LampInfoElement, LampState, PartState, StateElement,
};

pub const LAMP_SLOTS: usize = VIRTUAL_LAMP_COUNT as usize;

pub const PART_SLOTS: usize = GROUP_COUNT as usize;

pub trait LampSet: PartialEq {
    /// # Safety
    /// `place` is a place of `Self` that nothing reads before this call writes every field once.
    unsafe fn init(place: *mut Self);

    fn clear(&mut self);
}

#[derive(Debug, PartialEq, Eq)]
pub struct StateSet {
    lamps: [Option<LampState>; LAMP_SLOTS],
    parts: [Option<PartState>; PART_SLOTS],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldLampInfo {
    pub name: FixedText64,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeldIdentity {
    pub controller_id: FixedText32,
    pub discovery_prefix: FixedText32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct LampInfoSet {
    owner: Option<HeldIdentity>,
    lamps: [Option<HeldLampInfo>; LAMP_SLOTS],
}

impl LampSet for StateSet {
    unsafe fn init(place: *mut Self) {
        // SAFETY: the caller hands a place of `StateSet`; both arrays are written slot by slot, once.
        unsafe {
            write_empty(ptr::addr_of_mut!((*place).lamps).cast::<Option<LampState>>(), LAMP_SLOTS);
            write_empty(ptr::addr_of_mut!((*place).parts).cast::<Option<PartState>>(), PART_SLOTS);
        }
    }

    fn clear(&mut self) {
        self.lamps.iter_mut().for_each(|slot| *slot = None);
        self.parts.iter_mut().for_each(|slot| *slot = None);
    }
}

impl StateSet {
    pub(crate) fn take(&mut self, element: &StateElement) {
        match *element {
            StateElement::Lamp(lamp) => put(&mut self.lamps, lamp.lamp, lamp),
            StateElement::Part(part) => put(&mut self.parts, part.group, part),
        }
    }

    pub fn lamp(&self, lamp: u8) -> Option<LampState> {
        *self.lamps.get(usize::from(lamp))?
    }

    pub fn part(&self, group: u8) -> Option<PartState> {
        *self.parts.get(usize::from(group))?
    }

    pub fn lamp_ids(&self) -> u64 {
        held_mask(&self.lamps)
    }

    pub fn part_groups(&self) -> u16 {
        u16::try_from(held_mask(&self.parts)).unwrap_or(u16::MAX)
    }

    pub fn elements(&self) -> impl Iterator<Item = StateElement> + '_ {
        let lamps = self.lamps.iter().flatten().copied().map(StateElement::Lamp);
        lamps.chain(self.parts.iter().flatten().copied().map(StateElement::Part))
    }

    pub(crate) fn same_but_age(&self, other: &Self) -> bool {
        let lamps = self.lamps.iter().zip(other.lamps.iter()).all(|(mine, theirs)| {
            mine.map(|lamp| LampState { age_ms: None, ..lamp }) == theirs.map(|lamp| LampState { age_ms: None, ..lamp })
        });
        lamps && self.parts == other.parts
    }
}

impl LampSet for LampInfoSet {
    unsafe fn init(place: *mut Self) {
        // SAFETY: the caller hands a place of `LampInfoSet`; the owner and every lamp slot are written once.
        unsafe {
            ptr::addr_of_mut!((*place).owner).write(None);
            write_empty(ptr::addr_of_mut!((*place).lamps).cast::<Option<HeldLampInfo>>(), LAMP_SLOTS);
        }
    }

    fn clear(&mut self) {
        self.owner = None;
        self.lamps.iter_mut().for_each(|slot| *slot = None);
    }
}

impl LampInfoSet {
    pub(crate) fn take(&mut self, element: &LampInfoElement<'_>) {
        match element {
            LampInfoElement::Owner(owner) => self.owner = Some(HeldIdentity::from_wire(owner)),
            LampInfoElement::Lamp(lamp) => put(&mut self.lamps, lamp.lamp, HeldLampInfo::from_wire(lamp)),
        }
    }

    pub(crate) fn hold_owner(&mut self, owner: HeldIdentity) {
        self.owner = Some(owner);
    }

    pub(crate) fn hold_lamp(&mut self, lamp: u8, info: HeldLampInfo) {
        put(&mut self.lamps, lamp, info);
    }

    pub const fn owner(&self) -> Option<&HeldIdentity> {
        self.owner.as_ref()
    }

    pub fn lamp(&self, lamp: u8) -> Option<&HeldLampInfo> {
        self.lamps.get(usize::from(lamp))?.as_ref()
    }

    pub fn lamp_ids(&self) -> u64 {
        held_mask(&self.lamps)
    }

    pub fn elements(&self) -> impl Iterator<Item = LampInfoElement<'_>> + '_ {
        let owner = self.owner.iter().map(|owner| LampInfoElement::Owner(owner.to_wire()));
        let lamps = self.lamps.iter().zip(0_u8..).filter_map(|(slot, lamp)| {
            slot.as_ref().map(|info| LampInfoElement::Lamp(info.to_wire(lamp)))
        });
        owner.chain(lamps)
    }
}

impl StateSet {
    pub(crate) fn hold_lamp(&mut self, lamp: LampState) {
        put(&mut self.lamps, lamp.lamp, lamp);
    }

    pub(crate) fn hold_part(&mut self, part: PartState) {
        put(&mut self.parts, part.group, part);
    }
}

impl HeldIdentity {
    fn from_wire(owner: &HaIdentity<'_>) -> Self {
        Self { controller_id: fixed_text_32(owner.controller_id), discovery_prefix: fixed_text_32(owner.discovery_prefix) }
    }

    fn to_wire(&self) -> HaIdentity<'_> {
        HaIdentity { controller_id: self.controller_id.as_str(), discovery_prefix: self.discovery_prefix.as_str() }
    }
}

impl HeldLampInfo {
    fn from_wire(lamp: &LampInfo<'_>) -> Self {
        Self {
            name: fixed_text_64(lamp.name),
            device_type: lamp.device_type,
            device_type_overridden: lamp.device_type_overridden,
            device_types: lamp.device_types,
            color_mode: lamp.color_mode,
            color_mode_overridden: lamp.color_mode_overridden,
            capabilities: lamp.capabilities,
            kelvin_range: lamp.kelvin_range,
            bound: lamp.bound,
            ha_entity: lamp.ha_entity,
        }
    }

    fn to_wire(&self, lamp: u8) -> LampInfo<'_> {
        LampInfo {
            lamp,
            name: self.name.as_str(),
            device_type: self.device_type,
            device_type_overridden: self.device_type_overridden,
            device_types: self.device_types,
            color_mode: self.color_mode,
            color_mode_overridden: self.color_mode_overridden,
            capabilities: self.capabilities,
            kelvin_range: self.kelvin_range,
            bound: self.bound,
            ha_entity: self.ha_entity,
        }
    }
}

unsafe fn write_empty<T>(first: *mut Option<T>, slots: usize) {
    for index in 0..slots {
        // SAFETY: the caller hands the first of `slots` contiguous places of an array field.
        unsafe { first.add(index).write(None) };
    }
}

fn put<T>(slots: &mut [Option<T>], index: u8, value: T) {
    if let Some(slot) = slots.get_mut(usize::from(index)) {
        *slot = Some(value);
    }
}

fn held_mask<T>(slots: &[Option<T>]) -> u64 {
    slots.iter().zip(0_u32..).filter(|(slot, _)| slot.is_some()).fold(0, |mask, (_, index)| {
        mask | 1_u64.checked_shl(index).unwrap_or(0)
    })
}
