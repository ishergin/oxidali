use dali2rust_dali_phy::answer_table::addresses_the_device;
use dali2rust_platform::answer_table::{
    AnswerEntry, AnswerTableImage, COMPARE_ENTRY, NO_SHORT_ADDRESS, QUERY_SHORT_ADDRESS_ENTRY,
    VERIFY_SHORT_ADDRESS_ENTRY,
};
pub use dali2rust_platform::bus_unit::{BusUnitIdentity, BusUnitNvm, BusUnitSnapshot};

use crate::dali::dev103::address::{Device103Address, MAX_SHORT_ADDRESS};
use crate::dali::dev103::command::{Device103Command, Special103Command};
use crate::dali::dev103::event::power_notification;
use crate::dali::dev103::frame::ForwardFrame24;
use crate::dali::frame::send_twice_span_ms;

pub const ANSWER_YES: u8 = 0xFF;
pub const ANSWER_MASK: u8 = 0xFF;
pub const FACTORY_RANDOM_ADDRESS: u32 = 0x00FF_FFFF;
// IEC 62386-103 §9.14.2
pub const INITIALISATION_MS: u32 = 15 * 60 * 1_000;
// IEC 62386-103 §9.9.3
pub const QUIESCENT_MS: u32 = 15 * 60 * 1_000;
// IEC 62386-103 §11.4.2
pub const IDENTIFY_MS: u32 = 10_000;
// IEC 62386-103 §9.12.2
pub const POWER_NOTIFICATION_EARLIEST_MS: u32 = 1_300;
pub const POWER_NOTIFICATION_LATEST_MS: u32 = 4_500;
pub const SEND_TWICE_SPAN_MS: u32 = send_twice_span_ms(ForwardFrame24::DATA_BITS);
// IEC 62386-101 §4.2
pub const VERSION_2_0: u8 = 0x08;
pub const VERSION_NOT_IMPLEMENTED: u8 = 0xFF;

const RANDOM_ADDRESS_SPAN: u32 = 0x00FF_FFFF;
const DEVICE_INSTANCE: u8 = 0xFE;
const SPECIAL: u8 = Device103Address::Special(0).encode();
const DIRECT_WRITE_MEMORY: u8 = Device103Address::Special(2).encode();
const DTR1_DTR0: u8 = Device103Address::Special(3).encode();
const DTR2_DTR1: u8 = Device103Address::Special(4).encode();
const COMMAND_BIT: u8 = 0x01;
const INITIALISE_UNADDRESSED: u8 = 0x7F;
const INITIALISE_ALL: u8 = 0xFF;
const LAST_MEMORY_LOCATION: u8 = 0xFF;
const CAPABILITY_APPLICATION_CONTROLLER: u8 = 0x01;
pub const STANDARD_OPERATING_MODE: u8 = 0x00;
const MANUFACTURER_MODES_FROM: u8 = 0x80;
const UPPER_GROUPS_SHIFT: u32 = 16;
// IEC 62386-103 Table 15
const STATUS_QUIESCENT: u8 = 1 << 1;
const STATUS_NO_SHORT_ADDRESS: u8 = 1 << 2;
const STATUS_APPLICATION_ACTIVE: u8 = 1 << 3;
const STATUS_CONTROLLER_ERROR: u8 = 1 << 4;
const STATUS_POWER_CYCLE_SEEN: u8 = 1 << 5;
const STATUS_RESET_STATE: u8 = 1 << 6;
// IEC 62386-103 Table 12
const BANK0_LAST_LOCATION: u8 = 0x1A;
const BANK0_LOCATIONS: usize = BANK0_LAST_LOCATION as usize + 1;
const BANK0_FIRMWARE: usize = 0x09;
const BANK0_IDENTIFICATION: usize = 0x0B;
const BANK0_HARDWARE: usize = 0x13;
const BANK0_PART_VERSIONS: usize = 0x15;
const BANK0_LOGICAL_UNITS: usize = 0x18;
// IEC 62386-103 §11.10.21
const TESTFRAME_RESERVED: u8 = 0x80;
const TESTFRAME_TRANSACTION: u8 = 0x40;
const TESTFRAME_SIXTEEN_BIT: u8 = 0x20;
const TESTFRAME_REPEATS_SHIFT: u8 = 3;
const TESTFRAME_REPEATS: u8 = 0x03;
const TESTFRAME_PRIORITY: u8 = 0x07;
const TESTFRAME_PRIORITIES: core::ops::RangeInclusive<u8> = 1..=5;

// IEC 62386-103 Table 17
pub const FACTORY_NVM: BusUnitNvm = BusUnitNvm {
    short_address: None,
    random_address: FACTORY_RANDOM_ADDRESS,
    device_groups: 0,
    operating_mode: STANDARD_OPERATING_MODE,
    power_cycle_notification: false,
    application_active: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialisationState {
    Disabled,
    Enabled,
    Withdrawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitFrame {
    Forward24([u8; 3]),
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TestFrame {
    pub data: u8,
    pub dtr: [u8; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestFrameBits {
    Forward16([u8; 2]),
    Forward24([u8; 3]),
}

// IEC 62386-103 §11.10.21
impl TestFrame {
    #[must_use]
    pub const fn priority(self) -> u8 {
        self.data & TESTFRAME_PRIORITY
    }

    #[must_use]
    pub const fn repeats(self) -> u8 {
        (self.data >> TESTFRAME_REPEATS_SHIFT) & TESTFRAME_REPEATS
    }

    #[must_use]
    pub const fn in_transaction(self) -> bool {
        self.data & TESTFRAME_TRANSACTION != 0
    }

    #[must_use]
    pub const fn bits(self) -> TestFrameBits {
        let [dtr0, dtr1, dtr2] = self.dtr;
        if self.data & TESTFRAME_SIXTEEN_BIT != 0 {
            TestFrameBits::Forward16([dtr0, dtr1])
        } else {
            TestFrameBits::Forward24([dtr0, dtr1, dtr2])
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnitEffects {
    pub nvm_changed: bool,
    pub identify: Option<bool>,
    pub quiescent: Option<bool>,
    pub test_frame: Option<TestFrame>,
    pub power_notification: Option<[u8; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerOn {
    Pending,
    Notifying { at_ms: u32 },
    Done,
}

#[derive(Debug, Clone)]
pub struct Device103Unit {
    nvm: BusUnitNvm,
    revision: Option<u32>,
    wire_sequence: u32,
    bank0: [Option<u8>; BANK0_LOCATIONS],
    dtr: [Option<u8>; 3],
    search: [Option<u8>; 3],
    initialisation: InitialisationState,
    initialisation_until_ms: u32,
    quiescent_until_ms: Option<u32>,
    identify_until_ms: Option<u32>,
    write_enabled: bool,
    power_cycle_seen: bool,
    power_on: PowerOn,
    pending_twice: Option<([u8; 3], u32)>,
}

impl Device103Unit {
    // IEC 62386-103 §9.12.1
    #[must_use]
    pub fn new(identity: &BusUnitIdentity) -> Self {
        Self {
            nvm: FACTORY_NVM,
            revision: None,
            wire_sequence: 0,
            bank0: bank0(identity),
            dtr: [Some(0); 3],
            search: [Some(0xFF); 3],
            initialisation: InitialisationState::Disabled,
            initialisation_until_ms: 0,
            quiescent_until_ms: None,
            identify_until_ms: None,
            write_enabled: false,
            power_cycle_seen: true,
            power_on: PowerOn::Pending,
            pending_twice: None,
        }
    }

    #[must_use]
    pub const fn nvm(&self) -> BusUnitNvm {
        self.nvm
    }

    #[must_use]
    pub const fn is_hydrated(&self) -> bool {
        self.revision.is_some()
    }

    #[must_use]
    pub const fn quiescent(&self) -> bool {
        self.quiescent_until_ms.is_some()
    }

    #[must_use]
    pub fn quiescent_remaining_ms(&self, now_ms: u32) -> Option<u32> {
        self.quiescent_until_ms.and_then(|until| remaining_until(until, now_ms))
    }

    #[must_use]
    pub const fn initialisation(&self) -> InitialisationState {
        self.initialisation
    }

    #[must_use]
    pub const fn identifying(&self) -> bool {
        self.identify_until_ms.is_some()
    }

    #[must_use]
    pub const fn wire_sequence(&self) -> u32 {
        self.wire_sequence
    }

    #[must_use]
    pub const fn adopted_revision(&self) -> Option<u32> {
        self.revision
    }

    pub fn hydrate(&mut self, snapshot: &BusUnitSnapshot) -> bool {
        let newer = self
            .revision
            .is_none_or(|adopted| is_newer(snapshot.revision, adopted));
        if !newer || snapshot.acknowledged_wire_sequence != self.wire_sequence {
            return false;
        }
        self.revision = Some(snapshot.revision);
        self.nvm = BusUnitNvm {
            short_address: snapshot.nvm.short_address.filter(|address| *address <= MAX_SHORT_ADDRESS),
            ..snapshot.nvm
        };
        true
    }

    pub fn lost_capture(&mut self) {
        self.dtr = [None; 3];
        self.search = [None; 3];
        self.initialisation = InitialisationState::Disabled;
        self.write_enabled = false;
        self.pending_twice = None;
    }

    pub fn tick(&mut self, now_ms: u32, entropy: &mut dyn FnMut() -> u32) -> UnitEffects {
        let power_notification = self.power_on_step(now_ms, entropy);
        let mut effects = UnitEffects { power_notification, ..UnitEffects::default() };
        if self.initialising() && passed(now_ms, self.initialisation_until_ms) {
            self.initialisation = InitialisationState::Disabled;
        }
        if self.quiescent_until_ms.is_some_and(|until| passed(now_ms, until)) {
            self.quiescent_until_ms = None;
            effects.quiescent = Some(false);
        }
        if self.identify_until_ms.is_some_and(|until| passed(now_ms, until)) {
            self.identify_until_ms = None;
            effects.identify = Some(false);
        }
        effects
    }

    // IEC 62386-103 §9.12.2
    fn power_on_step(&mut self, now_ms: u32, entropy: &mut dyn FnMut() -> u32) -> Option<[u8; 3]> {
        match self.power_on {
            PowerOn::Pending if self.is_hydrated() => {
                let at_ms = now_ms.wrapping_add(power_notification_delay(entropy()));
                self.power_on = PowerOn::Notifying { at_ms };
                None
            }
            PowerOn::Notifying { at_ms } if passed(now_ms, at_ms) => {
                self.power_on = PowerOn::Done;
                let nvm = self.nvm;
                nvm.power_cycle_notification
                    .then(|| power_notification(nvm.short_address, nvm.device_groups).as_bytes())
            }
            _ => None,
        }
    }

    pub fn feed(
        &mut self,
        frame: UnitFrame,
        now_ms: u32,
        entropy: &mut dyn FnMut() -> u32,
    ) -> UnitEffects {
        let UnitFrame::Forward24(bytes) = frame else {
            self.pending_twice = None;
            return UnitEffects::default();
        };
        if !self.is_hydrated() {
            self.lost_capture();
            return UnitEffects::default();
        }
        let twice = self.second_of_pair(bytes, now_ms);
        let mut step = Step { unit: self, now_ms, twice, effects: UnitEffects::default() };
        step.frame(bytes, entropy);
        step.effects
    }

    // IEC 62386-101 §9.3
    fn second_of_pair(&mut self, bytes: [u8; 3], now_ms: u32) -> bool {
        let pair = matches!(
            self.pending_twice,
            Some((first, at)) if first == bytes && now_ms.wrapping_sub(at) < SEND_TWICE_SPAN_MS
        );
        self.pending_twice = if pair { None } else { Some((bytes, now_ms)) };
        pair
    }

    #[must_use]
    pub fn answer_table(&self) -> AnswerTableImage {
        let mut image = AnswerTableImage::SILENT;
        if !self.is_hydrated() {
            return image;
        }
        image.short_address = self.nvm.short_address.unwrap_or(NO_SHORT_ADDRESS);
        image.groups = self.nvm.device_groups;
        for command in Device103Command::ALL.iter().copied() {
            if command.metadata().expects_backward {
                image.set_device_query(command.metadata().opcode, self.query_entry(command));
            }
        }
        image.entries[COMPARE_ENTRY] = yes_if(self.compare());
        image.entries[VERIFY_SHORT_ADDRESS_ENTRY] =
            yes_if(self.initialising() && self.nvm.short_address.is_some());
        image.entries[QUERY_SHORT_ADDRESS_ENTRY] = if self.initialising() && self.selected() {
            byte(self.nvm.short_address.unwrap_or(ANSWER_MASK))
        } else {
            AnswerEntry::SILENT
        };
        image
    }

    fn query_entry(&self, command: Device103Command) -> AnswerEntry {
        self.status_entry(command)
            .or_else(|| self.register_entry(command))
            .or_else(|| self.mode_entry(command))
            .or_else(|| self.membership_entry(command))
            .unwrap_or(AnswerEntry::SILENT)
    }

    // IEC 62386-103 §11.6
    fn status_entry(&self, command: Device103Command) -> Option<AnswerEntry> {
        let status = self.status_byte();
        Some(match command {
            Device103Command::QueryDeviceStatus => {
                AnswerEntry::by_lease(Some(status), Some(status | STATUS_CONTROLLER_ERROR))
            }
            Device103Command::QueryApplicationControllerError => {
                AnswerEntry::by_lease(None, Some(ANSWER_MASK))
            }
            Device103Command::QueryMissingShortAddress => yes_if(self.nvm.short_address.is_none()),
            Device103Command::QueryVersionNumber => byte(VERSION_2_0),
            Device103Command::QueryNumberOfInstances => byte(0),
            _ => return None,
        })
    }

    // IEC 62386-103 §11.6
    fn register_entry(&self, command: Device103Command) -> Option<AnswerEntry> {
        let random = self.nvm.random_address.to_be_bytes();
        let known = match command {
            Device103Command::QueryContentDtr0 => self.dtr[0],
            Device103Command::QueryContentDtr1 => self.dtr[1],
            Device103Command::QueryContentDtr2 => self.dtr[2],
            Device103Command::QueryRandomAddressH => Some(random[1]),
            Device103Command::QueryRandomAddressM => Some(random[2]),
            Device103Command::QueryRandomAddressL => Some(random[3]),
            _ => return None,
        };
        Some(known.map_or(AnswerEntry::SILENT, byte))
    }

    // IEC 62386-103 §11.6
    fn mode_entry(&self, command: Device103Command) -> Option<AnswerEntry> {
        let mode = self.nvm.operating_mode;
        Some(match command {
            Device103Command::ReadMemoryLocation => {
                self.memory_answer().map_or(AnswerEntry::SILENT, byte)
            }
            Device103Command::QueryApplicationControlEnabled => {
                AnswerEntry::by_lease(self.nvm.application_active.then_some(ANSWER_YES), None)
            }
            Device103Command::QueryOperatingMode => byte(mode),
            Device103Command::QueryManufacturerSpecificMode => {
                yes_if(mode >= MANUFACTURER_MODES_FROM)
            }
            Device103Command::QueryQuiescentMode => yes_if(self.quiescent()),
            _ => return None,
        })
    }

    // IEC 62386-103 §11.6
    fn membership_entry(&self, command: Device103Command) -> Option<AnswerEntry> {
        let groups = self.nvm.device_groups.to_le_bytes();
        Some(match command {
            Device103Command::QueryDeviceGroups0To7 => byte(groups[0]),
            Device103Command::QueryDeviceGroups8To15 => byte(groups[1]),
            Device103Command::QueryDeviceGroups16To23 => byte(groups[2]),
            Device103Command::QueryDeviceGroups24To31 => byte(groups[3]),
            Device103Command::QueryPowerCycleNotification => {
                yes_if(self.nvm.power_cycle_notification)
            }
            Device103Command::QueryDeviceCapabilities => byte(CAPABILITY_APPLICATION_CONTROLLER),
            Device103Command::QueryResetState => yes_if(self.reset_state()),
            _ => return None,
        })
    }

    fn status_byte(&self) -> u8 {
        [
            (self.quiescent(), STATUS_QUIESCENT),
            (self.nvm.short_address.is_none(), STATUS_NO_SHORT_ADDRESS),
            (self.nvm.application_active, STATUS_APPLICATION_ACTIVE),
            (self.power_cycle_seen, STATUS_POWER_CYCLE_SEEN),
            (self.reset_state(), STATUS_RESET_STATE),
        ]
        .into_iter()
        .filter(|(set, _)| *set)
        .fold(0, |status, (_, bit)| status | bit)
    }

    // IEC 62386-103 §9.16.2.1
    const fn reset_state(&self) -> bool {
        self.nvm.device_groups == 0 && self.nvm.random_address == FACTORY_RANDOM_ADDRESS
    }

    // IEC 62386-103 §9.10.4
    fn memory_answer(&self) -> Option<u8> {
        let (Some(0), Some(offset)) = (self.dtr[1], self.dtr[0]) else {
            return None;
        };
        if offset > BANK0_LAST_LOCATION {
            return None;
        }
        self.bank0.get(usize::from(offset)).copied().flatten()
    }

    fn initialising(&self) -> bool {
        self.initialisation != InitialisationState::Disabled
    }

    fn search_address(&self) -> Option<u32> {
        let [Some(h), Some(m), Some(l)] = self.search else {
            return None;
        };
        Some(u32::from_be_bytes([0, h, m, l]))
    }

    fn selected(&self) -> bool {
        self.search_address() == Some(self.nvm.random_address)
    }

    // IEC 62386-103 §11.10.5
    fn compare(&self) -> bool {
        self.initialisation == InitialisationState::Enabled
            && self.search_address().is_some_and(|search| self.nvm.random_address <= search)
    }

    // IEC 62386-103 §9.5.1
    fn accepts(&self, address: u8) -> bool {
        address & COMMAND_BIT != 0
            && addresses_the_device(
                address,
                self.nvm.short_address.unwrap_or(NO_SHORT_ADDRESS),
                self.nvm.device_groups,
            )
    }
}

struct Step<'a> {
    unit: &'a mut Device103Unit,
    now_ms: u32,
    twice: bool,
    effects: UnitEffects,
}

impl Step<'_> {
    fn frame(&mut self, [address, instance, data]: [u8; 3], entropy: &mut dyn FnMut() -> u32) {
        match address {
            SPECIAL => {
                if let Some(command) = Special103Command::from_selector(instance) {
                    self.special(command, data, entropy);
                }
            }
            DIRECT_WRITE_MEMORY => {
                self.stop_identification();
                self.direct_write(instance);
            }
            DTR1_DTR0 => {
                self.stop_identification();
                self.unit.dtr[..2].copy_from_slice(&[Some(data), Some(instance)]);
            }
            DTR2_DTR1 => {
                self.stop_identification();
                self.unit.dtr[1..].copy_from_slice(&[Some(data), Some(instance)]);
            }
            _ if instance == DEVICE_INSTANCE && self.unit.accepts(address) => {
                if let Some(command) = Device103Command::from_opcode(data) {
                    self.device(command);
                }
            }
            _ => {}
        }
    }

    // IEC 62386-103 §9.10.5
    fn device(&mut self, command: Device103Command) {
        if !matches!(
            command,
            Device103Command::QueryContentDtr0
                | Device103Command::QueryContentDtr1
                | Device103Command::QueryContentDtr2
        ) {
            self.unit.write_enabled = false;
        }
        if command.metadata().expects_backward {
            self.query(command);
        } else if self.twice {
            if command != Device103Command::IdentifyDevice {
                self.stop_identification();
            }
            if !self.control(command) {
                self.configure(command);
            }
        }
    }

    // IEC 62386-103 §11.6.15
    fn query(&mut self, command: Device103Command) {
        if command != Device103Command::ReadMemoryLocation {
            return;
        }
        self.unit.dtr[0] = match self.unit.dtr[1] {
            Some(0) => self.unit.dtr[0].map(step_location),
            Some(_) => self.unit.dtr[0],
            None => None,
        };
    }

    // IEC 62386-103 §11.4, §11.5
    fn control(&mut self, command: Device103Command) -> bool {
        match command {
            Device103Command::IdentifyDevice => {
                self.unit.identify_until_ms = Some(self.now_ms.wrapping_add(IDENTIFY_MS));
                self.effects.identify = Some(true);
            }
            Device103Command::ResetPowerCycleSeen => self.unit.power_cycle_seen = false,
            Device103Command::Reset => self.reset(),
            Device103Command::SetShortAddress => {
                if let Some(address) = self.unit.dtr[0].and_then(short_address_operand) {
                    self.set_nvm(|nvm| nvm.short_address = address);
                }
            }
            Device103Command::EnableWriteMemory => self.unit.write_enabled = true,
            Device103Command::SetOperatingMode => self.operating_mode(),
            _ => return false,
        }
        true
    }

    // IEC 62386-103 §11.5
    fn configure(&mut self, command: Device103Command) {
        match command {
            Device103Command::EnableApplicationController => {
                self.set_nvm(|nvm| nvm.application_active = true);
            }
            Device103Command::DisableApplicationController => {
                self.set_nvm(|nvm| nvm.application_active = false);
            }
            Device103Command::StartQuiescentMode => self.quiescent(true),
            Device103Command::StopQuiescentMode => self.quiescent(false),
            Device103Command::EnablePowerCycleNotification => {
                self.set_nvm(|nvm| nvm.power_cycle_notification = true);
            }
            Device103Command::DisablePowerCycleNotification => {
                self.set_nvm(|nvm| nvm.power_cycle_notification = false);
            }
            _ => self.groups(command),
        }
    }

    // IEC 62386-103 §11.5.2
    fn reset(&mut self) {
        self.set_nvm(|nvm| {
            nvm.device_groups = 0;
            nvm.random_address = FACTORY_RANDOM_ADDRESS;
        });
        self.unit.search = [Some(0xFF); 3];
        self.unit.write_enabled = false;
        self.unit.power_cycle_seen = false;
        self.quiescent(false);
    }

    // IEC 62386-103 §11.5.8
    fn operating_mode(&mut self) {
        if self.unit.dtr[0] == Some(STANDARD_OPERATING_MODE) {
            self.set_nvm(|nvm| nvm.operating_mode = STANDARD_OPERATING_MODE);
        }
    }

    // IEC 62386-103 §11.5
    fn groups(&mut self, command: Device103Command) {
        let (shift, add) = match command {
            Device103Command::AddToDeviceGroups0To15 => (0, true),
            Device103Command::AddToDeviceGroups16To31 => (UPPER_GROUPS_SHIFT, true),
            Device103Command::RemoveFromDeviceGroups0To15 => (0, false),
            Device103Command::RemoveFromDeviceGroups16To31 => (UPPER_GROUPS_SHIFT, false),
            _ => return,
        };
        let (Some(low), Some(high)) = (self.unit.dtr[1], self.unit.dtr[2]) else {
            return;
        };
        let bits = u32::from(u16::from_le_bytes([low, high])) << shift;
        self.set_nvm(|nvm| {
            nvm.device_groups = if add { nvm.device_groups | bits } else { nvm.device_groups & !bits };
        });
    }

    fn quiescent(&mut self, start: bool) {
        if start || self.unit.quiescent() {
            self.effects.quiescent = Some(start);
        }
        if start {
            self.unit.power_on = PowerOn::Done;
        }
        self.unit.quiescent_until_ms = start.then(|| self.now_ms.wrapping_add(QUIESCENT_MS));
    }

    fn special(
        &mut self,
        command: Special103Command,
        data: u8,
        entropy: &mut dyn FnMut() -> u32,
    ) {
        if !keeps_write_enabled(command) {
            self.unit.write_enabled = false;
        }
        if stops_identification(command) && (self.twice || !command.send_twice()) {
            self.stop_identification();
        }
        match command {
            Special103Command::Terminate => {
                self.unit.initialisation = InitialisationState::Disabled;
            }
            Special103Command::Initialise => self.initialise(data),
            Special103Command::Randomise => self.randomise(entropy),
            Special103Command::WriteMemoryLocation | Special103Command::WriteMemoryLocationNoReply => {
                self.write_memory();
            }
            Special103Command::Dtr0 => self.unit.dtr[0] = Some(data),
            Special103Command::Dtr1 => self.unit.dtr[1] = Some(data),
            Special103Command::Dtr2 => self.unit.dtr[2] = Some(data),
            Special103Command::SendTestframe => self.test_frame(data),
            _ => self.search(command, data),
        }
    }

    // IEC 62386-103 §11.10.3
    fn initialise(&mut self, data: u8) {
        if !self.twice || !self.initialise_addresses_us(data) {
            return;
        }
        if self.unit.initialisation == InitialisationState::Disabled {
            self.unit.initialisation = InitialisationState::Enabled;
        }
        self.unit.initialisation_until_ms = self.now_ms.wrapping_add(INITIALISATION_MS);
    }

    // IEC 62386-103 Table 23
    fn initialise_addresses_us(&self, data: u8) -> bool {
        match (data, self.unit.nvm.short_address) {
            (INITIALISE_ALL, _) | (INITIALISE_UNADDRESSED, None) => true,
            (address, Some(short)) => address == short,
            _ => false,
        }
    }

    // IEC 62386-103 §11.10.4
    fn randomise(&mut self, entropy: &mut dyn FnMut() -> u32) {
        if !self.twice || !self.unit.initialising() {
            return;
        }
        let random = entropy() % RANDOM_ADDRESS_SPAN;
        self.set_nvm(|nvm| nvm.random_address = random);
    }

    // IEC 62386-103 §11.10
    fn search(&mut self, command: Special103Command, data: u8) {
        if !self.unit.initialising() {
            return;
        }
        match command {
            Special103Command::Withdraw => self.withdraw(),
            Special103Command::SearchAddrH => self.unit.search[0] = Some(data),
            Special103Command::SearchAddrM => self.unit.search[1] = Some(data),
            Special103Command::SearchAddrL => self.unit.search[2] = Some(data),
            Special103Command::ProgramShortAddress if self.unit.selected() => {
                if let Some(address) = short_address_operand(data) {
                    self.set_nvm(|nvm| nvm.short_address = address);
                }
            }
            _ => {}
        }
    }

    // IEC 62386-103 §11.10.6
    fn withdraw(&mut self) {
        if self.unit.selected() {
            self.unit.initialisation = InitialisationState::Withdrawn;
        }
    }

    // IEC 62386-103 §11.10.13
    fn write_memory(&mut self) {
        if self.unit.write_enabled && self.unit.dtr[1] == Some(0) {
            self.unit.dtr[0] = self.unit.dtr[0].map(step_location);
        }
    }

    // IEC 62386-103 §11.10.18
    fn direct_write(&mut self, offset: u8) {
        if self.unit.write_enabled && self.unit.dtr[1] == Some(0) {
            self.unit.dtr[0] = Some(step_location(offset));
        }
    }

    // IEC 62386-103 §11.10.21, §9.9.3
    fn test_frame(&mut self, data: u8) {
        if data & TESTFRAME_RESERVED != 0 || !TESTFRAME_PRIORITIES.contains(&(data & TESTFRAME_PRIORITY)) {
            return;
        }
        if self.unit.quiescent() {
            return;
        }
        let [Some(dtr0), Some(dtr1), dtr2] = self.unit.dtr else {
            return;
        };
        let unused_by_a_16_bit_frame = (data & TESTFRAME_SIXTEEN_BIT != 0).then_some(0);
        let Some(dtr2) = dtr2.or(unused_by_a_16_bit_frame) else {
            return;
        };
        self.effects.test_frame = Some(TestFrame { data, dtr: [dtr0, dtr1, dtr2] });
    }

    // IEC 62386-103 §9.14.3
    fn stop_identification(&mut self) {
        if self.unit.identify_until_ms.take().is_some() {
            self.effects.identify = Some(false);
        }
    }

    fn set_nvm(&mut self, change: impl FnOnce(&mut BusUnitNvm)) {
        let before = self.unit.nvm;
        change(&mut self.unit.nvm);
        if self.unit.nvm != before {
            self.unit.wire_sequence = self.unit.wire_sequence.wrapping_add(1);
            self.effects.nvm_changed = true;
        }
    }
}

// IEC 62386-103 §9.14.3
const fn stops_identification(command: Special103Command) -> bool {
    !matches!(
        command,
        Special103Command::Initialise
            | Special103Command::Compare
            | Special103Command::VerifyShortAddress
            | Special103Command::QueryShortAddress
    )
}

// IEC 62386-103 §9.10.5
const fn keeps_write_enabled(command: Special103Command) -> bool {
    matches!(
        command,
        Special103Command::WriteMemoryLocation
            | Special103Command::WriteMemoryLocationNoReply
            | Special103Command::Dtr0
            | Special103Command::Dtr1
            | Special103Command::Dtr2
    )
}

const fn passed(now_ms: u32, until_ms: u32) -> bool {
    now_ms.wrapping_sub(until_ms) < u32::MAX / 2
}

const fn power_notification_delay(draw: u32) -> u32 {
    let span = POWER_NOTIFICATION_LATEST_MS - POWER_NOTIFICATION_EARLIEST_MS + 1;
    POWER_NOTIFICATION_EARLIEST_MS + draw % span
}

#[must_use]
pub const fn remaining_until(until_ms: u32, now_ms: u32) -> Option<u32> {
    if passed(now_ms, until_ms) {
        None
    } else {
        Some(until_ms.wrapping_sub(now_ms))
    }
}

pub const fn is_newer(candidate: u32, adopted: u32) -> bool {
    let ahead = candidate.wrapping_sub(adopted);
    ahead != 0 && ahead < u32::MAX / 2
}

// IEC 62386-103 §9.10.4
const fn step_location(offset: u8) -> u8 {
    if offset < LAST_MEMORY_LOCATION {
        offset + 1
    } else {
        offset
    }
}

// IEC 62386-103 §9.14.1
const fn short_address_operand(value: u8) -> Option<Option<u8>> {
    match value {
        ANSWER_MASK => Some(None),
        address if address <= MAX_SHORT_ADDRESS => Some(Some(address)),
        _ => None,
    }
}

const fn yes_if(condition: bool) -> AnswerEntry {
    if condition {
        AnswerEntry::always(ANSWER_YES)
    } else {
        AnswerEntry::SILENT
    }
}

const fn byte(answer: u8) -> AnswerEntry {
    AnswerEntry::always(answer)
}

fn bank0(identity: &BusUnitIdentity) -> [Option<u8>; BANK0_LOCATIONS] {
    let mut bank = [Some(0); BANK0_LOCATIONS];
    bank[0] = Some(BANK0_LAST_LOCATION);
    bank[1] = None;
    let parts = [VERSION_2_0, VERSION_NOT_IMPLEMENTED, VERSION_2_0];
    for (at, bytes) in [
        (BANK0_FIRMWARE, &identity.firmware_version[..]),
        (BANK0_IDENTIFICATION, &identity.identification_number[..]),
        (BANK0_HARDWARE, &identity.hardware_version[..]),
        (BANK0_PART_VERSIONS, &parts[..]),
        (BANK0_LOGICAL_UNITS, &[1, 0, 0][..]),
    ] {
        for (slot, value) in bank[at..at + bytes.len()].iter_mut().zip(bytes) {
            *slot = Some(*value);
        }
    }
    bank
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dali::dev103::address::InstanceAddress;
    use dali2rust_dali_phy::{AnswerTableCell, TableLookup};

    const NOW: u32 = 1_000;
    const GAP_MS: u32 = 25;
    const ENTROPY: u32 = 0x0012_3456;
    const IDENTITY: BusUnitIdentity = BusUnitIdentity {
        firmware_version: [1, 4],
        hardware_version: [1, 0],
        identification_number: [0, 0, 0x30, 0xED, 0xA0, 0x12, 0x34, 0x56],
    };

    struct Bench {
        unit: Device103Unit,
        now_ms: u32,
        effects: UnitEffects,
    }

    impl Bench {
        fn hydrated(nvm: BusUnitNvm) -> Self {
            let mut unit = Device103Unit::new(&IDENTITY);
            assert!(unit.hydrate(&snapshot(nvm, 1, 0)));
            Self { unit, now_ms: NOW, effects: UnitEffects::default() }
        }

        fn send(&mut self, bytes: [u8; 3]) -> &mut Self {
            self.now_ms = self.now_ms.wrapping_add(GAP_MS);
            let effects = self.unit.feed(UnitFrame::Forward24(bytes), self.now_ms, &mut || ENTROPY);
            self.effects = merge(self.effects, effects);
            self
        }

        fn twice(&mut self, bytes: [u8; 3]) -> &mut Self {
            self.send(bytes).send(bytes)
        }

        fn device(&mut self, address: Device103Address, command: Device103Command) -> &mut Self {
            let bytes = command.frame(address).as_bytes();
            if command.metadata().send_twice {
                self.twice(bytes)
            } else {
                self.send(bytes)
            }
        }

        fn special(&mut self, command: Special103Command, data: u8) -> &mut Self {
            let bytes = command.frame(data).as_bytes();
            if command.send_twice() {
                self.twice(bytes)
            } else {
                self.send(bytes)
            }
        }

        fn ask(&self, bytes: [u8; 3]) -> TableLookup {
            ask_table(&self.unit.answer_table(), bytes, true)
        }

        fn query(&self, address: Device103Address, command: Device103Command) -> TableLookup {
            self.ask(command.frame(address).as_bytes())
        }

        fn broadcast(&self, command: Device103Command) -> TableLookup {
            self.query(Device103Address::Broadcast, command)
        }

        fn read_bank0(&mut self, offset: u8) -> TableLookup {
            self.special(Special103Command::Dtr1, 0).special(Special103Command::Dtr0, offset);
            self.broadcast(Device103Command::ReadMemoryLocation)
        }
    }

    fn merge(old: UnitEffects, new: UnitEffects) -> UnitEffects {
        UnitEffects {
            nvm_changed: old.nvm_changed || new.nvm_changed,
            identify: new.identify.or(old.identify),
            quiescent: new.quiescent.or(old.quiescent),
            test_frame: new.test_frame.or(old.test_frame),
            power_notification: new.power_notification.or(old.power_notification),
        }
    }

    fn ask_table(image: &AnswerTableImage, bytes: [u8; 3], lease_alive: bool) -> TableLookup {
        let cell = AnswerTableCell::new();
        cell.publish(image, 0, NOW);
        cell.set_lease_until_tick(if lease_alive { NOW + 1 } else { NOW });
        cell.lookup(bytes, 0, NOW)
    }

    fn snapshot(nvm: BusUnitNvm, revision: u32, acknowledged_wire_sequence: u32) -> BusUnitSnapshot {
        BusUnitSnapshot { nvm, revision, acknowledged_wire_sequence, acknowledged_quiescence: 0 }
    }

    fn at(short: u8) -> BusUnitNvm {
        BusUnitNvm { short_address: Some(short), ..FACTORY_NVM }
    }

    fn yes() -> TableLookup {
        TableLookup::Answer(ANSWER_YES)
    }

    fn select(bench: &mut Bench, random: u32) {
        let [_, h, m, l] = random.to_be_bytes();
        bench
            .special(Special103Command::SearchAddrH, h)
            .special(Special103Command::SearchAddrM, m)
            .special(Special103Command::SearchAddrL, l);
    }

    fn commissioned() -> Bench {
        let mut bench = Bench::hydrated(FACTORY_NVM);
        bench.special(Special103Command::Initialise, INITIALISE_ALL);
        bench.special(Special103Command::Randomise, 0);
        bench
    }

    #[test]
    fn an_unhydrated_unit_answers_nothing_and_executes_nothing() {
        let mut unit = Device103Unit::new(&IDENTITY);
        let capabilities = Device103Command::QueryDeviceCapabilities
            .frame(Device103Address::Broadcast)
            .as_bytes();
        assert_eq!(ask_table(&unit.answer_table(), capabilities, true), TableLookup::Silent);
        let disable = Device103Command::DisableApplicationController
            .frame(Device103Address::Broadcast)
            .as_bytes();
        unit.feed(UnitFrame::Forward24(disable), NOW, &mut || ENTROPY);
        assert!(unit.hydrate(&snapshot(FACTORY_NVM, 1, 0)));
        let second = unit.feed(UnitFrame::Forward24(disable), NOW + 1, &mut || ENTROPY);
        assert!(!second.nvm_changed, "the first half reached a unit that could not act on it");
        assert!(unit.nvm().application_active);
    }

    #[test]
    fn a_frame_before_hydration_leaves_the_registers_it_may_have_moved_unknown() {
        let dtr0 = Device103Command::QueryContentDtr0.frame(Device103Address::Broadcast).as_bytes();
        let mut quiet = Device103Unit::new(&IDENTITY);
        assert!(quiet.hydrate(&snapshot(FACTORY_NVM, 1, 0)));
        assert_eq!(ask_table(&quiet.answer_table(), dtr0, true), TableLookup::Answer(0));
        let mut unit = Device103Unit::new(&IDENTITY);
        let set = Special103Command::Dtr0.frame(5).as_bytes();
        unit.feed(UnitFrame::Forward24(set), NOW, &mut || ENTROPY);
        assert!(unit.hydrate(&snapshot(FACTORY_NVM, 1, 0)));
        assert_eq!(ask_table(&unit.answer_table(), dtr0, true), TableLookup::Silent);
    }

    // IEC 62386-103 §11.6.16
    fn qace_reply(nvm: BusUnitNvm, address: Device103Address, lease_alive: bool) -> TableLookup {
        let configured = nvm.short_address.filter(|short| *short <= MAX_SHORT_ADDRESS);
        let addressed = match address {
            Device103Address::Broadcast => true,
            Device103Address::BroadcastUnaddressed => configured.is_none(),
            Device103Address::Short(short) => configured == Some(short),
            Device103Address::Group(_) | Device103Address::Special(_) => false,
        };
        match (addressed, nvm.application_active, lease_alive) {
            (false, _, _) => TableLookup::NotAddressed,
            (true, false, _) => TableLookup::Silent,
            (true, true, true) => TableLookup::Answer(ANSWER_YES),
            (true, true, false) => TableLookup::LeaseLapsed,
        }
    }

    #[test]
    fn application_control_enabled_is_answered_yes_only_while_active_and_leased() {
        let qace = Device103Command::QueryApplicationControlEnabled;
        let addresses = [
            Device103Address::Broadcast,
            Device103Address::BroadcastUnaddressed,
            Device103Address::Short(0),
            Device103Address::Short(7),
            Device103Address::Short(63),
        ];
        for active in [true, false] {
            for short in [None, Some(0), Some(7), Some(63), Some(200)] {
                let nvm = BusUnitNvm { short_address: short, application_active: active, ..FACTORY_NVM };
                let table = Bench::hydrated(nvm).unit.answer_table();
                for address in addresses {
                    for lease in [true, false] {
                        assert_eq!(
                            ask_table(&table, qace.frame(address).as_bytes(), lease),
                            qace_reply(nvm, address, lease),
                            "{active} {short:?} {address:?} lease {lease}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_configuration_instruction_runs_only_on_its_second_identical_frame() {
        let mut bench = Bench::hydrated(at(7));
        let set = Device103Command::SetShortAddress.frame(Device103Address::Short(7)).as_bytes();
        bench.special(Special103Command::Dtr0, 9).send(set);
        assert_eq!(bench.unit.nvm().short_address, Some(7), "one frame is not a pair");
        bench.send(set);
        assert_eq!(bench.unit.nvm().short_address, Some(9));
        assert!(bench.effects.nvm_changed);
    }

    #[test]
    fn any_frame_between_the_two_halves_splits_the_pair() {
        let mut bench = Bench::hydrated(at(7));
        let set = Device103Command::SetShortAddress.frame(Device103Address::Short(7)).as_bytes();
        bench.special(Special103Command::Dtr0, 9).send(set);
        bench.unit.feed(UnitFrame::Other, bench.now_ms, &mut || ENTROPY);
        bench.send(set);
        assert_eq!(bench.unit.nvm().short_address, Some(7), "a backward frame between splits it");
        bench.send(set);
        assert_eq!(bench.unit.nvm().short_address, Some(9), "the second frame opened a new pair");
    }

    #[test]
    fn a_pair_is_judged_by_its_settling_time_not_by_the_distance_of_its_stamps() {
        let set = Device103Command::SetShortAddress.frame(Device103Address::Short(7)).as_bytes();
        for (apart_ms, paired) in [(115, true), (SEND_TWICE_SPAN_MS - 1, true), (SEND_TWICE_SPAN_MS, false)] {
            let mut bench = Bench::hydrated(at(7));
            bench.special(Special103Command::Dtr0, 9).send(set);
            bench.now_ms += apart_ms - GAP_MS;
            bench.send(set);
            let expected = if paired { Some(9) } else { Some(7) };
            assert_eq!(bench.unit.nvm().short_address, expected, "{apart_ms} ms from stamp to stamp");
        }
    }

    #[test]
    fn three_identical_frames_execute_once() {
        let mut bench = commissioned();
        let randomise = Special103Command::Randomise.frame(0).as_bytes();
        let mut draws = 0;
        for _ in 0..3 {
            bench.now_ms += GAP_MS;
            bench.unit.feed(UnitFrame::Forward24(randomise), bench.now_ms, &mut || {
                draws += 1;
                ENTROPY
            });
        }
        assert_eq!(draws, 1, "the third frame opens a new pair and draws nothing");
    }

    #[test]
    fn only_the_addressing_the_unit_matches_reaches_it() {
        let mut bench = Bench::hydrated(at(7));
        let disable = Device103Command::DisableApplicationController;
        bench.device(Device103Address::Short(8), disable);
        bench.device(Device103Address::BroadcastUnaddressed, disable);
        bench.device(Device103Address::Group(3), disable);
        assert!(bench.unit.nvm().application_active);
        bench.special(Special103Command::Dtr1, 0b1000).special(Special103Command::Dtr2, 0);
        bench.device(Device103Address::Short(7), Device103Command::AddToDeviceGroups0To15);
        bench.device(Device103Address::Group(3), disable);
        assert!(!bench.unit.nvm().application_active);
        assert_eq!(bench.query(Device103Address::Group(3), Device103Command::QueryDeviceGroups0To7), TableLookup::Answer(0b1000));
        assert_eq!(bench.query(Device103Address::Group(4), Device103Command::QueryDeviceGroups0To7), TableLookup::NotAddressed);
    }

    #[test]
    fn an_unaddressed_broadcast_reaches_a_unit_without_a_short_address() {
        let mut bench = Bench::hydrated(FACTORY_NVM);
        bench.device(Device103Address::BroadcastUnaddressed, Device103Command::DisableApplicationController);
        assert!(!bench.unit.nvm().application_active);
        assert_eq!(bench.query(Device103Address::BroadcastUnaddressed, Device103Command::QueryMissingShortAddress), yes());
    }

    #[test]
    fn another_master_finds_and_addresses_the_unit_by_its_random_address() {
        let mut bench = commissioned();
        assert_eq!(bench.unit.nvm().random_address, ENTROPY % RANDOM_ADDRESS_SPAN);
        assert!(bench.effects.nvm_changed);
        assert_eq!(bench.broadcast(Device103Command::QueryResetState), TableLookup::Silent, "a random address is not the reset value");
        let compare = Special103Command::Compare.frame(0).as_bytes();
        select(&mut bench, ENTROPY - 1);
        assert_eq!(bench.ask(compare), TableLookup::Silent, "random above search: NO");
        select(&mut bench, ENTROPY);
        assert_eq!(bench.ask(compare), yes());
        let query_short = Special103Command::QueryShortAddress.frame(0).as_bytes();
        assert_eq!(bench.ask(query_short), TableLookup::Answer(ANSWER_MASK));
        bench.special(Special103Command::ProgramShortAddress, 12);
        assert_eq!(bench.unit.nvm().short_address, Some(12));
        assert_eq!(bench.ask(Special103Command::VerifyShortAddress.frame(12).as_bytes()), yes());
        assert_eq!(bench.ask(Special103Command::VerifyShortAddress.frame(11).as_bytes()), TableLookup::Silent);
        bench.special(Special103Command::Withdraw, 0);
        assert_eq!(bench.unit.initialisation(), InitialisationState::Withdrawn);
        assert_eq!(bench.ask(compare), TableLookup::Silent, "a withdrawn unit no longer compares");
        assert_eq!(bench.ask(query_short), TableLookup::Answer(12));
        bench.special(Special103Command::Terminate, 0);
        assert_eq!(bench.ask(query_short), TableLookup::Silent);
    }

    #[test]
    fn randomise_needs_its_pair_and_program_needs_the_unit_selected() {
        let mut bench = Bench::hydrated(FACTORY_NVM);
        bench.special(Special103Command::Initialise, INITIALISE_ALL);
        bench.send(Special103Command::Randomise.frame(0).as_bytes());
        assert_eq!(bench.unit.nvm().random_address, FACTORY_RANDOM_ADDRESS, "one frame is not a pair");
        select(&mut bench, FACTORY_RANDOM_ADDRESS - 1);
        bench.special(Special103Command::ProgramShortAddress, 3);
        bench.special(Special103Command::Withdraw, 0);
        assert_eq!(bench.unit.nvm().short_address, None, "another unit's search address");
        assert_eq!(bench.unit.initialisation(), InitialisationState::Enabled);
        select(&mut bench, FACTORY_RANDOM_ADDRESS);
        bench.special(Special103Command::ProgramShortAddress, 3);
        assert_eq!(bench.unit.nvm().short_address, Some(3));
    }

    #[test]
    fn initialisation_commands_are_ignored_outside_the_initialisation_state() {
        let mut bench = Bench::hydrated(FACTORY_NVM);
        bench.special(Special103Command::Randomise, 0);
        select(&mut bench, FACTORY_RANDOM_ADDRESS);
        bench.special(Special103Command::ProgramShortAddress, 3);
        assert_eq!(bench.unit.nvm(), FACTORY_NVM);
        assert_eq!(bench.ask(Special103Command::Compare.frame(0).as_bytes()), TableLookup::Silent);
        assert_eq!(bench.ask(Special103Command::VerifyShortAddress.frame(0xFF).as_bytes()), TableLookup::Silent);
    }

    #[test]
    fn initialise_selects_by_short_address_mask_or_all() {
        for (short, data, enters) in [
            (Some(5), 5, true),
            (Some(5), 6, false),
            (Some(5), INITIALISE_UNADDRESSED, false),
            (None, INITIALISE_UNADDRESSED, true),
            (None, 5, false),
            (Some(5), INITIALISE_ALL, true),
            (Some(5), 0x80, false),
        ] {
            let mut bench = Bench::hydrated(BusUnitNvm { short_address: short, ..FACTORY_NVM });
            bench.send(Special103Command::Initialise.frame(data).as_bytes());
            assert_eq!(bench.unit.initialisation(), InitialisationState::Disabled, "one frame is not a pair");
            bench.send(Special103Command::Initialise.frame(data).as_bytes());
            let entered = bench.unit.initialisation() == InitialisationState::Enabled;
            assert_eq!(entered, enters, "{short:?} {data:#04x}");
        }
    }

    #[test]
    fn the_initialisation_state_ends_fifteen_minutes_after_the_last_initialise() {
        let mut bench = Bench::hydrated(FACTORY_NVM);
        bench.special(Special103Command::Initialise, INITIALISE_ALL);
        bench.now_ms += INITIALISATION_MS / 2;
        bench.special(Special103Command::Initialise, INITIALISE_ALL);
        let restarted = bench.now_ms;
        bench.unit.tick(restarted + INITIALISATION_MS - 1, &mut || ENTROPY);
        assert_eq!(bench.unit.initialisation(), InitialisationState::Enabled, "the second INITIALISE restarted it");
        bench.unit.tick(restarted + INITIALISATION_MS, &mut || ENTROPY);
        assert_eq!(bench.unit.initialisation(), InitialisationState::Disabled);
    }

    #[test]
    fn memory_bank_0_reads_with_auto_increment_and_answers_no_for_holes() {
        let mut bench = Bench::hydrated(at(1));
        assert_eq!(bench.read_bank0(0x00), TableLookup::Answer(0x1A), "test 12.6.1 takes 0x1A or 0x80–0xFE");
        assert_eq!(bench.read_bank0(0x01), TableLookup::Silent, "location 1 is not implemented");
        for (offset, expected) in [(0x09, 1), (0x0A, 4), (0x0D, 0x30), (0x12, 0x56), (0x15, VERSION_2_0), (0x16, VERSION_NOT_IMPLEMENTED), (0x17, VERSION_2_0), (0x18, 1)] {
            assert_eq!(bench.read_bank0(offset), TableLookup::Answer(expected), "{offset:#04x}");
        }
        let read = Device103Command::ReadMemoryLocation.frame(Device103Address::Broadcast).as_bytes();
        bench.read_bank0(0x09);
        bench.send(read);
        assert_eq!(bench.ask(read), TableLookup::Answer(4), "DTR0 moved on after the read");
        assert_eq!(bench.read_bank0(0x1B), TableLookup::Silent, "reserved from 0x1B");
        bench.send(read);
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(0x1C));
        bench.special(Special103Command::Dtr0, LAST_MEMORY_LOCATION).send(read);
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(0xFF));
    }

    #[test]
    fn a_bank_the_unit_does_not_have_is_ignored_and_dtr0_stays() {
        let mut bench = Bench::hydrated(at(1));
        bench.special(Special103Command::Dtr1, 1).special(Special103Command::Dtr0, 3);
        let read = Device103Command::ReadMemoryLocation.frame(Device103Address::Broadcast).as_bytes();
        assert_eq!(bench.ask(read), TableLookup::Silent);
        bench.send(read);
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(3));
    }

    #[test]
    fn the_status_byte_reports_real_state_and_the_lease_raises_the_error_bit() {
        let mut bench = Bench::hydrated(at(2));
        let expected = STATUS_APPLICATION_ACTIVE | STATUS_POWER_CYCLE_SEEN | STATUS_RESET_STATE;
        let status = Device103Command::QueryDeviceStatus.frame(Device103Address::Short(2)).as_bytes();
        assert_eq!(bench.ask(status), TableLookup::Answer(expected));
        assert_eq!(
            ask_table(&bench.unit.answer_table(), status, false),
            TableLookup::Answer(expected | STATUS_CONTROLLER_ERROR)
        );
        let error = Device103Command::QueryApplicationControllerError.frame(Device103Address::Short(2)).as_bytes();
        assert_eq!(bench.ask(error), TableLookup::Silent, "no error: NO");
        assert_eq!(ask_table(&bench.unit.answer_table(), error, false), TableLookup::Answer(ANSWER_MASK));
        bench.device(Device103Address::Short(2), Device103Command::ResetPowerCycleSeen);
        bench.device(Device103Address::Short(2), Device103Command::StartQuiescentMode);
        assert_eq!(bench.ask(status), TableLookup::Answer(expected - STATUS_POWER_CYCLE_SEEN + STATUS_QUIESCENT));
    }

    #[test]
    fn the_capabilities_name_an_application_controller_without_instances() {
        let bench = Bench::hydrated(FACTORY_NVM);
        assert_eq!(bench.broadcast(Device103Command::QueryDeviceCapabilities), TableLookup::Answer(1));
        assert_eq!(bench.broadcast(Device103Command::QueryNumberOfInstances), TableLookup::Answer(0));
        assert_eq!(bench.broadcast(Device103Command::QueryVersionNumber), TableLookup::Answer(VERSION_2_0));
        assert_eq!(bench.broadcast(Device103Command::QueryInputDeviceError), TableLookup::Silent);
        assert_eq!(bench.broadcast(Device103Command::QueryExtendedVersionNumber), TableLookup::Silent);
    }

    #[test]
    fn quiescent_mode_starts_stops_and_lapses_after_fifteen_minutes() {
        let mut bench = Bench::hydrated(at(2));
        bench.device(Device103Address::Broadcast, Device103Command::StartQuiescentMode);
        assert!(bench.unit.quiescent());
        assert_eq!(bench.effects.quiescent, Some(true));
        assert_eq!(bench.broadcast(Device103Command::QueryQuiescentMode), yes());
        let started = bench.now_ms;
        assert_eq!(bench.unit.tick(started + QUIESCENT_MS - 1, &mut || ENTROPY).quiescent, None);
        assert_eq!(bench.unit.tick(started + QUIESCENT_MS, &mut || ENTROPY).quiescent, Some(false));
        bench.device(Device103Address::Broadcast, Device103Command::StartQuiescentMode);
        bench.device(Device103Address::Broadcast, Device103Command::StopQuiescentMode);
        assert!(!bench.unit.quiescent());
        assert_eq!(bench.effects.quiescent, Some(false));
    }

    #[test]
    fn every_start_reports_and_moves_the_deadline_to_fifteen_minutes_from_itself() {
        let mut bench = Bench::hydrated(at(2));
        bench.device(Device103Address::Broadcast, Device103Command::StartQuiescentMode);
        let first = bench.now_ms;
        assert_eq!(bench.unit.quiescent_remaining_ms(first), Some(QUIESCENT_MS));
        bench.now_ms = first + QUIESCENT_MS / 2;
        bench.effects = UnitEffects::default();
        bench.device(Device103Address::Broadcast, Device103Command::StartQuiescentMode);
        let second = bench.now_ms;
        assert_eq!(bench.effects.quiescent, Some(true));
        assert_eq!(bench.unit.quiescent_remaining_ms(second), Some(QUIESCENT_MS));
        assert_eq!(bench.unit.tick(first + QUIESCENT_MS, &mut || ENTROPY).quiescent, None);
        assert_eq!(bench.unit.quiescent_remaining_ms(second + QUIESCENT_MS - 1), Some(1));
        assert_eq!(bench.unit.quiescent_remaining_ms(second + QUIESCENT_MS), None);
    }

    #[test]
    fn a_stop_outside_quiescent_mode_reports_nothing() {
        let mut bench = Bench::hydrated(at(2));
        bench.device(Device103Address::Broadcast, Device103Command::StopQuiescentMode);
        assert_eq!(bench.effects.quiescent, None);
        assert_eq!(bench.unit.quiescent_remaining_ms(bench.now_ms), None);
    }

    #[test]
    fn device_groups_add_and_remove_by_dtr2_dtr1() {
        let mut bench = Bench::hydrated(at(2));
        let to = Device103Address::Short(2);
        let pair = |bench: &mut Bench, high: u8, low: u8| {
            bench.send(ForwardFrame24::from_bytes([DTR2_DTR1, high, low]).as_bytes());
        };
        pair(&mut bench, 0x80, 0x01);
        bench.device(to, Device103Command::AddToDeviceGroups16To31);
        bench.device(to, Device103Command::AddToDeviceGroups0To15);
        assert_eq!(bench.unit.nvm().device_groups, 0x8001_8001);
        pair(&mut bench, 0x00, 0x01);
        bench.device(to, Device103Command::RemoveFromDeviceGroups16To31);
        assert_eq!(bench.unit.nvm().device_groups, 0x8000_8001);
        assert_eq!(bench.broadcast(Device103Command::QueryDeviceGroups24To31), TableLookup::Answer(0x80));
        assert_eq!(bench.broadcast(Device103Command::QueryResetState), TableLookup::Silent);
    }

    #[test]
    fn a_lost_capture_silences_what_depends_on_the_dtrs_and_the_search_until_they_are_set() {
        let mut bench = commissioned();
        select(&mut bench, ENTROPY);
        bench.special(Special103Command::Dtr0, 4);
        bench.unit.lost_capture();
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Silent);
        assert_eq!(bench.ask(Special103Command::Compare.frame(0).as_bytes()), TableLookup::Silent);
        bench.device(Device103Address::Broadcast, Device103Command::SetShortAddress);
        assert_eq!(bench.unit.nvm().short_address, None, "an unknown DTR0 is not an operand");
        assert_eq!(bench.unit.initialisation(), InitialisationState::Disabled, "the lost frame may have been TERMINATE");
        bench.special(Special103Command::Initialise, INITIALISE_ALL);
        select(&mut bench, ENTROPY);
        assert_eq!(bench.ask(Special103Command::Compare.frame(0).as_bytes()), yes());
    }

    #[test]
    fn a_lost_capture_drops_write_enable() {
        let mut bench = Bench::hydrated(at(2));
        bench.device(Device103Address::Short(2), Device103Command::EnableWriteMemory);
        bench.unit.lost_capture();
        bench.special(Special103Command::Dtr1, 0).special(Special103Command::Dtr0, 0x10);
        bench.special(Special103Command::WriteMemoryLocation, 0xAA);
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(0x10));
    }

    #[test]
    fn a_read_with_an_unknown_bank_leaves_dtr0_unknown() {
        let mut bench = Bench::hydrated(at(2));
        bench.special(Special103Command::Dtr0, 5);
        bench.unit.lost_capture();
        bench.special(Special103Command::Dtr0, 5);
        bench.send(Device103Command::ReadMemoryLocation.frame(Device103Address::Broadcast).as_bytes());
        assert_eq!(
            bench.broadcast(Device103Command::QueryContentDtr0),
            TableLookup::Silent,
            "DTR1 may have been 0, so DTR0 may have moved on"
        );
    }

    #[test]
    fn verify_short_address_is_answered_only_by_an_addressed_unit() {
        let verify_mask = Special103Command::VerifyShortAddress.frame(ANSWER_MASK).as_bytes();
        let unaddressed = commissioned();
        assert_eq!(unaddressed.ask(verify_mask), TableLookup::Silent, "test 12.7.9 step 7");
        let mut addressed = Bench::hydrated(at(5));
        addressed.special(Special103Command::Initialise, INITIALISE_ALL);
        let verify_own = Special103Command::VerifyShortAddress.frame(5).as_bytes();
        assert_eq!(addressed.ask(verify_own), yes());
    }

    #[test]
    fn a_first_hydration_needs_the_sequence_of_a_fresh_unit() {
        let mut unit = Device103Unit::new(&IDENTITY);
        assert!(!unit.hydrate(&snapshot(at(3), 1, 1)), "the acknowledged sequence is per boot");
        assert!(unit.hydrate(&snapshot(at(3), 1, 0)));
    }

    #[test]
    fn a_short_address_out_of_range_in_a_snapshot_is_no_address() {
        let bench = Bench::hydrated(at(200));
        assert_eq!(bench.unit.nvm().short_address, None);
        assert_eq!(bench.broadcast(Device103Command::QueryMissingShortAddress), yes());
    }

    #[test]
    fn set_short_address_ignores_an_operand_out_of_range() {
        let mut bench = Bench::hydrated(at(2));
        bench.special(Special103Command::Dtr0, 64);
        bench.device(Device103Address::Short(2), Device103Command::SetShortAddress);
        bench.special(Special103Command::Dtr0, 254);
        bench.device(Device103Address::Short(2), Device103Command::SetShortAddress);
        assert_eq!(bench.unit.nvm().short_address, Some(2));
    }

    #[test]
    fn the_dtr_pair_frames_and_direct_write_keep_write_enable() {
        let mut bench = Bench::hydrated(at(2));
        bench.device(Device103Address::Short(2), Device103Command::EnableWriteMemory);
        for (frame, dtr0) in [
            ([DTR1_DTR0, 0, 0x30], 0x31),
            ([DTR2_DTR1, 0, 0], 0x32),
            ([DIRECT_WRITE_MEMORY, 0x40, 0xAA], 0x42),
        ] {
            bench.send(frame);
            bench.special(Special103Command::WriteMemoryLocation, 0xAA);
            assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(dtr0), "{frame:02X?}");
        }
    }

    #[test]
    fn a_snapshot_is_adopted_only_when_it_is_newer_and_has_seen_every_wire_change() {
        let mut bench = Bench::hydrated(at(2));
        bench.special(Special103Command::Dtr0, 9);
        bench.device(Device103Address::Short(2), Device103Command::SetShortAddress);
        let seen = bench.unit.wire_sequence();
        assert_eq!(seen, 1);
        assert!(!bench.unit.hydrate(&snapshot(at(2), 2, 0)), "newer, but taken before the wire change");
        assert_eq!(bench.unit.nvm().short_address, Some(9));
        assert!(bench.unit.hydrate(&snapshot(at(9), 3, seen)));
        assert!(!bench.unit.hydrate(&snapshot(at(2), 2, seen)), "an older revision after a newer one");
        assert!(!bench.unit.hydrate(&snapshot(at(5), 3, seen)), "the revision already adopted");
        assert!(bench.unit.hydrate(&snapshot(at(4), 4, seen)), "an operator's change after the wire's");
        assert_eq!(bench.unit.nvm().short_address, Some(4));
        assert!(bench.unit.hydrate(&snapshot(at(6), 4u32.wrapping_add(u32::MAX / 2 - 1), seen)));
    }

    #[test]
    fn a_command_that_changes_nothing_moves_no_wire_sequence() {
        let mut bench = Bench::hydrated(at(2));
        bench.device(Device103Address::Short(2), Device103Command::EnableApplicationController);
        assert_eq!(bench.unit.wire_sequence(), 0);
        assert!(!bench.effects.nvm_changed);
    }

    #[test]
    fn writing_memory_needs_write_enable_and_any_other_command_drops_it() {
        let mut bench = Bench::hydrated(at(2));
        bench.special(Special103Command::Dtr1, 0).special(Special103Command::Dtr0, 0x10);
        bench.special(Special103Command::WriteMemoryLocation, 0xAA);
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(0x10));
        bench.device(Device103Address::Short(2), Device103Command::EnableWriteMemory);
        bench.special(Special103Command::WriteMemoryLocation, 0xAA);
        bench.special(Special103Command::WriteMemoryLocationNoReply, 0xAA);
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(0x12));
        bench.send(ForwardFrame24::from_bytes([DIRECT_WRITE_MEMORY, 0x20, 0xAA]).as_bytes());
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(0x21));
        let status = Device103Command::QueryDeviceStatus.frame(Device103Address::Short(2)).as_bytes();
        bench.send(status);
        bench.special(Special103Command::WriteMemoryLocation, 0xAA);
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(0x21));
    }

    #[test]
    fn identification_runs_ten_seconds_and_any_other_instruction_stops_it() {
        let mut bench = Bench::hydrated(at(2));
        let identify = |bench: &mut Bench| {
            bench.effects = UnitEffects::default();
            bench.device(Device103Address::Short(2), Device103Command::IdentifyDevice);
            assert_eq!(bench.effects.identify, Some(true));
        };
        identify(&mut bench);
        let started = bench.now_ms;
        assert_eq!(bench.unit.tick(started + IDENTIFY_MS, &mut || ENTROPY).identify, Some(false));
        identify(&mut bench);
        bench.special(Special103Command::Initialise, INITIALISE_ALL);
        bench.special(Special103Command::Compare, 0);
        bench.send(Device103Command::QueryDeviceStatus.frame(Device103Address::Short(2)).as_bytes());
        assert!(bench.unit.identifying(), "INITIALISE and queries leave it running");
        bench.special(Special103Command::Dtr0, 1);
        assert_eq!(bench.effects.identify, Some(false));
        assert!(!bench.unit.identifying());
        for stop in [Special103Command::Terminate, Special103Command::SearchAddrH] {
            identify(&mut bench);
            bench.special(stop, 0);
            assert!(!bench.unit.identifying(), "{stop:?}");
        }
        identify(&mut bench);
        bench.send(ForwardFrame24::from_bytes([DTR1_DTR0, 0, 0]).as_bytes());
        assert!(!bench.unit.identifying(), "DTR1:DTR0");
    }

    #[test]
    fn send_testframe_carries_the_dtrs_it_reads() {
        let mut bench = Bench::hydrated(at(2));
        bench.special(Special103Command::Dtr0, 1).special(Special103Command::Dtr1, 2).special(Special103Command::Dtr2, 3);
        bench.special(Special103Command::SendTestframe, 0x05);
        assert_eq!(bench.effects.test_frame, Some(TestFrame { data: 0x05, dtr: [1, 2, 3] }));
    }

    #[test]
    fn send_testframe_refuses_a_reserved_bit_or_a_priority_outside_1_to_5() {
        for data in [0x85, 0x00, 0x06, 0x07] {
            let mut bench = Bench::hydrated(at(2));
            bench.special(Special103Command::Dtr0, 1).special(Special103Command::Dtr1, 2).special(Special103Command::Dtr2, 3);
            bench.special(Special103Command::SendTestframe, data);
            assert_eq!(bench.effects.test_frame, None, "{data:#04x}");
        }
        let mut bench = Bench::hydrated(at(2));
        bench.special(Special103Command::Dtr0, 1).special(Special103Command::Dtr1, 2).special(Special103Command::Dtr2, 3);
        bench.special(Special103Command::SendTestframe, 0x01);
        assert!(bench.effects.test_frame.is_some(), "priority 1 is in range");
    }

    #[test]
    fn no_testframe_is_requested_while_another_controller_holds_the_bus_quiescent() {
        let mut bench = Bench::hydrated(at(2));
        bench.special(Special103Command::Dtr0, 1).special(Special103Command::Dtr1, 2).special(Special103Command::Dtr2, 3);
        bench.device(Device103Address::Broadcast, Device103Command::StartQuiescentMode);
        bench.special(Special103Command::SendTestframe, 0x05);
        assert_eq!(bench.effects.test_frame, None);
        bench.device(Device103Address::Broadcast, Device103Command::StopQuiescentMode);
        bench.special(Special103Command::SendTestframe, 0x05);
        assert_eq!(bench.effects.test_frame, Some(TestFrame { data: 0x05, dtr: [1, 2, 3] }));
    }

    #[test]
    fn a_16_bit_testframe_needs_only_the_dtrs_it_sends() {
        let mut bench = Bench::hydrated(at(2));
        bench.unit.lost_capture();
        bench.special(Special103Command::Dtr0, 0xA5).special(Special103Command::Dtr1, 0x5A);
        bench.special(Special103Command::SendTestframe, 0x05);
        assert_eq!(bench.effects.test_frame, None, "a 24-bit frame still needs DTR2");
        bench.special(Special103Command::SendTestframe, 0x25);
        assert_eq!(bench.effects.test_frame, Some(TestFrame { data: 0x25, dtr: [0xA5, 0x5A, 0] }));
    }

    #[test]
    fn a_testframe_s_data_names_its_width_priority_repeats_and_transaction() {
        let in_transaction = TestFrame { data: 0b0101_0011, dtr: [1, 2, 3] };
        assert_eq!(in_transaction.priority(), 3);
        assert_eq!(in_transaction.repeats(), 2);
        assert!(in_transaction.in_transaction());
        assert_eq!(in_transaction.bits(), TestFrameBits::Forward24([1, 2, 3]));

        let sixteen = TestFrame { data: 0b0011_1101, dtr: [4, 5, 6] };
        assert_eq!(sixteen.priority(), 5);
        assert_eq!(sixteen.repeats(), 3);
        assert!(!sixteen.in_transaction());
        assert_eq!(sixteen.bits(), TestFrameBits::Forward16([4, 5]));
    }

    #[test]
    fn only_the_standard_operating_mode_is_accepted() {
        let mut bench = Bench::hydrated(at(2));
        bench.special(Special103Command::Dtr0, MANUFACTURER_MODES_FROM);
        bench.device(Device103Address::Short(2), Device103Command::SetOperatingMode);
        assert_eq!(bench.broadcast(Device103Command::QueryOperatingMode), TableLookup::Answer(0));
        assert_eq!(bench.broadcast(Device103Command::QueryManufacturerSpecificMode), TableLookup::Silent);
    }

    #[test]
    fn reset_restores_table_17_and_keeps_the_initialisation_as_test_12_7_1_expects() {
        let mut bench = commissioned();
        let to = Device103Address::Broadcast;
        bench.special(Special103Command::Dtr1, 1).special(Special103Command::Dtr2, 0);
        bench.device(to, Device103Command::AddToDeviceGroups0To15);
        bench.special(Special103Command::Dtr0, 6);
        bench.device(to, Device103Command::SetShortAddress);
        for command in [
            Device103Command::DisableApplicationController,
            Device103Command::EnablePowerCycleNotification,
            Device103Command::StartQuiescentMode,
            Device103Command::EnableWriteMemory,
        ] {
            bench.device(to, command);
        }
        bench.special(Special103Command::Dtr2, 7);
        bench.device(to, Device103Command::Reset);
        let nvm = bench.unit.nvm();
        assert_eq!(
            nvm,
            BusUnitNvm {
                short_address: Some(6),
                random_address: FACTORY_RANDOM_ADDRESS,
                device_groups: 0,
                operating_mode: STANDARD_OPERATING_MODE,
                power_cycle_notification: true,
                application_active: false,
            }
        );
        assert_eq!(bench.unit.initialisation(), InitialisationState::Enabled);
        assert!(!bench.unit.quiescent());
        assert_eq!(bench.broadcast(Device103Command::QueryResetState), yes());
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr2), TableLookup::Answer(7), "DTRs: no change");
        let status = bench.broadcast(Device103Command::QueryDeviceStatus);
        assert_eq!(status, TableLookup::Answer(STATUS_RESET_STATE), "powerCycleSeen resets to FALSE");
        bench.special(Special103Command::Dtr1, 0).special(Special103Command::Dtr0, 0x10);
        bench.special(Special103Command::WriteMemoryLocation, 0xAA);
        assert_eq!(bench.broadcast(Device103Command::QueryContentDtr0), TableLookup::Answer(0x10), "writeEnableState: DISABLED");
        bench.special(Special103Command::Randomise, 0);
        assert_eq!(bench.unit.nvm().random_address, ENTROPY % RANDOM_ADDRESS_SPAN, "RANDOMISE still runs after RESET");
    }

    #[test]
    fn instance_commands_and_event_frames_never_reach_the_unit() {
        let mut bench = Bench::hydrated(at(2));
        let instance = ForwardFrame24::command(Device103Address::Short(2), InstanceAddress::Number(0), 0x17);
        bench.twice(instance.as_bytes());
        let event_frame = [Device103Address::Short(2).encode() & !COMMAND_BIT, DEVICE_INSTANCE, 0x17];
        bench.twice(event_frame);
        assert!(bench.unit.nvm().application_active);
    }

    const NOTIFYING: BusUnitNvm = BusUnitNvm {
        short_address: Some(5),
        device_groups: 0b0110_0100,
        power_cycle_notification: true,
        ..FACTORY_NVM
    };
    const NAMED_NOTIFICATION: [u8; 3] = [0xFE, 0xF1, 0x45];
    const LAST_DRAW: u32 = POWER_NOTIFICATION_LATEST_MS - POWER_NOTIFICATION_EARLIEST_MS;

    fn power_on(nvm: BusUnitNvm, draw: u32) -> (Bench, u32) {
        let mut bench = Bench::hydrated(nvm);
        let completed = bench.now_ms;
        let armed = bench.unit.tick(completed, &mut || draw);
        assert_eq!(armed.power_notification, None, "the delay starts when the power-on procedure ends");
        (bench, completed)
    }

    fn notification_at(bench: &mut Bench, now_ms: u32) -> Option<[u8; 3]> {
        bench.unit.tick(now_ms, &mut || ENTROPY).power_notification
    }

    // IEC 62386-103 §9.12.2
    #[test]
    fn a_power_notification_waits_its_drawn_delay_and_is_sent_once() {
        for (draw, delay) in [
            (0, POWER_NOTIFICATION_EARLIEST_MS),
            (LAST_DRAW, POWER_NOTIFICATION_LATEST_MS),
            (LAST_DRAW + 1, POWER_NOTIFICATION_EARLIEST_MS),
        ] {
            let (mut bench, completed) = power_on(NOTIFYING, draw);
            assert_eq!(notification_at(&mut bench, completed + delay - 1), None, "draw {draw}");
            assert_eq!(notification_at(&mut bench, completed + delay), Some(NAMED_NOTIFICATION), "draw {draw}");
            assert_eq!(notification_at(&mut bench, completed + delay + 1), None, "sent once");
        }
    }

    #[test]
    fn a_unit_at_its_factory_notification_setting_announces_no_power_cycle() {
        let (mut bench, completed) = power_on(FACTORY_NVM, 0);
        assert_eq!(notification_at(&mut bench, completed + POWER_NOTIFICATION_LATEST_MS), None);
    }

    // IEC 62386-103 §9.9.3
    #[test]
    fn quiescent_mode_started_inside_the_delay_suppresses_the_notification_for_good() {
        let (mut bench, completed) = power_on(NOTIFYING, LAST_DRAW);
        bench
            .device(Device103Address::Broadcast, Device103Command::StartQuiescentMode)
            .device(Device103Address::Broadcast, Device103Command::StopQuiescentMode);
        assert_eq!(notification_at(&mut bench, completed + POWER_NOTIFICATION_LATEST_MS), None);
    }

    #[test]
    fn a_notification_disabled_inside_the_delay_is_not_sent() {
        let (mut bench, completed) = power_on(NOTIFYING, LAST_DRAW);
        bench.device(Device103Address::Broadcast, Device103Command::DisablePowerCycleNotification);
        assert_eq!(notification_at(&mut bench, completed + POWER_NOTIFICATION_LATEST_MS), None);
    }

    #[test]
    fn a_later_hydration_is_not_a_power_cycle() {
        let (mut bench, completed) = power_on(NOTIFYING, 0);
        let sent_at = completed + POWER_NOTIFICATION_EARLIEST_MS;
        assert_eq!(notification_at(&mut bench, sent_at), Some(NAMED_NOTIFICATION));
        assert!(bench.unit.hydrate(&snapshot(NOTIFYING, 2, 0)));
        assert_eq!(notification_at(&mut bench, sent_at + 1), None);
        assert_eq!(notification_at(&mut bench, sent_at + POWER_NOTIFICATION_LATEST_MS + 1), None);
    }

    #[test]
    fn nothing_is_armed_before_the_unit_is_hydrated() {
        let mut unit = Device103Unit::new(&IDENTITY);
        assert_eq!(unit.tick(NOW, &mut || 0).power_notification, None);
        assert!(unit.hydrate(&snapshot(NOTIFYING, 1, 0)));
        let completed = NOW + POWER_NOTIFICATION_LATEST_MS;
        assert_eq!(unit.tick(completed, &mut || 0).power_notification, None, "the delay starts here");
        let early = completed + POWER_NOTIFICATION_EARLIEST_MS - 1;
        assert_eq!(unit.tick(early, &mut || 0).power_notification, None);
    }
}
