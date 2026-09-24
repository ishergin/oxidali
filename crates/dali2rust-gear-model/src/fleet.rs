use dali2rust_domain::dali::banks::part251::LuminaireFormat;
use dali2rust_domain::dali::commands::DaliCommand;
use dali2rust_domain::dali::devices::dt8_color::{colour_value_is_wide, 
    Dt8Command, GEAR_FEATURES_AUTOMATIC_ACTIVATION, GEAR_FEATURES_STORE_RESERVED_MASK,
};
use dali2rust_domain::dali::devices::dt6_led::Dt6Command;
use dali2rust_domain::dali::devices::{DeviceCommandMetadata, DeviceType};
use dali2rust_domain::dali::net::address::decode_wire_address;
use dali2rust_domain::dali::pres::codec::dali_command_from_wire;
use dali2rust_domain::dali::pres::extended::ExtendedCommand;
use dali2rust_domain::dali::pres::opcode::READ_MEMORY_LOCATION_OPCODE;
use dali2rust_domain::dali::pres::special::SpecialCommand;
use dali2rust_domain::dali::pres::standard::StandardCommand;
use dali2rust_domain::dali::types::DaliAddress;
use dali2rust_platform::dali::TransferOutcome;

use crate::gear::{
    apply_standard_write, gear_query_reply, Gear, GearSpec, DALI_YES, DAPC_MASK_LEVEL,
    SHORT_ADDRESS_MASK,
    TC_COOLEST_MIREK, TC_WARMEST_MIREK,
};
use crate::rng::Rng;

pub const DEFAULT_RESERVED_SHORT_ADDRESSES: u64 = 0x0000_0000_0000_03FF;

#[derive(Debug, Default)]
struct BusRegisters {
    dtr0: u8,
    dtr1: u8,
    dtr2: u8,
    search_address: u32,
    enabled_device_type: Option<(u8, u64)>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FleetStats {
    pub reserved_conflicts: u32,
    pub refused_programs: u32,
    pub dropped_answers: u32,
    pub enable_device_type_consumed_by_interloper: u32,
    pub send_twice_split_by_interloper: u32,
    pub send_twice_pairs_executed: u32,
    pub dt8_gear_features_reserved_bits: u32,
}

#[derive(Debug)]
pub struct GearFleet {
    gears: Vec<Gear>,
    regs: BusRegisters,
    rng: Rng,
    reserved: u64,
    stats: FleetStats,
    answers: Vec<u8>,
    frame_seq: u64,
}

impl GearFleet {
    pub fn new(specs: Vec<GearSpec>, reserved: u64, rng_seed: u32) -> Self {
        let mut stats = FleetStats::default();
        let gears = specs
            .into_iter()
            .map(|mut spec| {
                if is_reserved(reserved, spec.short_address) {
                    stats.reserved_conflicts += 1;
                    spec.short_address = None;
                }
                Gear::new(spec)
            })
            .collect();
        Self {
            gears,
            regs: BusRegisters::default(),
            rng: Rng::new(rng_seed),
            reserved,
            stats,
            frame_seq: 0,
            answers: Vec::new(),
        }
    }

    pub fn demo_bus() -> Self {
        let mut specs: Vec<GearSpec> = (0..4)
            .map(|short| GearSpec::dt6(Some(short), demo_random_address(short)))
            .collect();
        specs.push(GearSpec::dt8(
            Some(4),
            demo_random_address(4),
            (true, false, false),
            (153, 370),
        ));
        specs.push(GearSpec::dt8(
            Some(5),
            demo_random_address(5),
            (false, true, true),
            (160, 400),
        ));
        specs.push(GearSpec::dt6(None, demo_random_address(6)));
        let mut six = GearSpec::dt8(Some(7), demo_random_address(7), (false, true, true), (160, 400));
        six.rgbwaf_channels = 6;
        specs.push(six);
        specs.push(
            GearSpec::dt8(Some(8), demo_random_address(8), (true, false, false), (153, 370))
                .with_metering(true, true),
        );
        let mut asset =
            GearSpec::dt8(Some(9), demo_random_address(9), (true, false, false), (153, 370));
        asset.luminaire_format = Some(LuminaireFormat::V3);
        asset.bus_unit_extension = true;
        specs.push(asset);
        let mut fleet = Self::new(specs, 0, DEMO_RNG_SEED);
        if let Some(gear) = fleet
            .gears_mut()
            .iter_mut()
            .find(|g| g.spec.short_address == Some(8))
        {
            if let Some(banks) = gear.metering.as_mut() {
                banks.implement_loadside(0, false);
            }
        }
        fleet
    }

    pub fn gears(&self) -> &[Gear] {
        &self.gears
    }

    pub fn gears_mut(&mut self) -> &mut [Gear] {
        &mut self.gears
    }

    pub fn stats(&self) -> FleetStats {
        self.stats
    }

    pub fn any_gear_holds(&self, frame: u16) -> bool {
        self.gears.iter().any(|gear| gear.holds(frame))
    }

    pub fn reserved(&self) -> u64 {
        self.reserved
    }

    pub fn last_answers(&self) -> &[u8] {
        &self.answers
    }

    fn publish(&mut self, buf: Vec<u8>) -> TransferOutcome {
        let outcome = merge_answers(&buf);
        self.answers = buf;
        outcome
    }

    fn publish_one(&mut self, value: Option<u8>) -> TransferOutcome {
        let mut buf = core::mem::take(&mut self.answers);
        buf.clear();
        buf.extend(value);
        self.publish(buf)
    }

    fn take_scratch(&mut self) -> Vec<u8> {
        let mut buf = core::mem::take(&mut self.answers);
        buf.clear();
        buf
    }

    pub fn expects_backward(&self, frame: u16) -> bool {
        let wire_address = (frame >> 8) as u8;
        let opcode = frame as u8;
        match dali_command_from_wire(wire_address, opcode, 0) {
            Ok(DaliCommand::Special(special)) => special.expects_backward(),
            Ok(DaliCommand::Standard { command, .. }) => command.is_query(),
            _ => {
                if wire_address & 0x01 == 0 {
                    return false;
                }
                opcode == READ_MEMORY_LOCATION_OPCODE
                    || (self.armed_device_type() == Some(DeviceType::Color.code())
                        && (is_dt8_query_opcode(opcode)
                            || opcode == QUERY_EXTENDED_VERSION_NUMBER_OPCODE))
                    || (self.armed_device_type() == Some(DeviceType::Led.code())
                        && Dt6Command::from_opcode(opcode)
                            .is_some_and(|c| c.metadata().expects_backward))
            }
        }
    }

    pub fn exchange(&mut self, frame: u16, expects_backward: bool) -> TransferOutcome {
        self.frame_seq = self.frame_seq.saturating_add(1);
        let wire_address = (frame >> 8) as u8;
        let opcode = frame as u8;
        let decoded = dali_command_from_wire(wire_address, opcode, 0);
        let repeats = self.frame_requires_repeat(decoded.as_ref().ok(), wire_address, opcode);
        self.hear(frame, decoded.as_ref().ok(), repeats);
        if let Ok(DaliCommand::Special(special)) = decoded {
            return self.on_special(special);
        }
        let enabled = self.take_enabled_device_type();
        self.carry_prelude_over_held_half(frame, enabled);
        let Ok(address) = decode_wire_address(wire_address) else {
            return self.publish_one(None);
        };
        if wire_address & 0x01 == 0 {
            return self.on_dapc(address, opcode);
        }
        if expects_backward && opcode == READ_MEMORY_LOCATION_OPCODE {
            return self.on_read_memory_location(address);
        }
        if enabled == Some(DeviceType::Led.code()) && is_dt6_opcode(opcode) {
            return self.on_extended_dt6(address, opcode, expects_backward);
        }
        if is_dt8_opcode(opcode)
            || (enabled == Some(DeviceType::Color.code())
                && opcode == QUERY_EXTENDED_VERSION_NUMBER_OPCODE)
        {
            return self.on_extended_dt8(address, opcode, expects_backward, enabled);
        }
        match decoded {
            Ok(DaliCommand::Standard { command, .. }) => self.on_standard(address, command),
            _ => self.publish_one(None),
        }
    }

    fn armed_device_type(&self) -> Option<u8> {
        self.regs.enabled_device_type.map(|(device_type, _)| device_type)
    }

    fn fresh_device_type(&self) -> Option<u8> {
        let (device_type, armed_seq) = self.regs.enabled_device_type?;
        (armed_seq + 1 == self.frame_seq).then_some(device_type)
    }

    fn take_enabled_device_type(&mut self) -> Option<u8> {
        let fresh = self.fresh_device_type();
        self.regs.enabled_device_type = None;
        fresh
    }

    fn frame_requires_repeat(
        &self,
        decoded: Option<&DaliCommand>,
        wire_address: u8,
        opcode: u8,
    ) -> bool {
        requires_repeat_under(decoded, wire_address, opcode, self.fresh_device_type())
    }

    #[cfg(test)]
    pub(crate) fn frame_pairs_now(&self, frame: u16) -> bool {
        let wire_address = (frame >> 8) as u8;
        let opcode = frame as u8;
        let decoded = dali_command_from_wire(wire_address, opcode, 0);
        requires_repeat_under(
            decoded.as_ref().ok(),
            wire_address,
            opcode,
            self.armed_device_type(),
        )
    }

    fn hear(&mut self, frame: u16, decoded: Option<&DaliCommand>, repeats: bool) {
        let audience = audience_of(decoded);
        let mut split = 0u32;
        let mut pairs = 0u32;
        for gear in &mut self.gears {
            let heard = match audience {
                Some(Audience::Every) if gear.enabled => gear.hear(frame, repeats),
                Some(Audience::Addressed(address)) if gear.matches(address) => {
                    gear.hear(frame, repeats)
                }
                _ => {
                    gear.executes_now = false;
                    continue;
                }
            };
            gear.executes_now = heard.executes;
            // IEC 62386-102 §11.5.13
            if !is_walk_query(decoded) {
                gear.cancel_device_type_walk();
            }
            split += u32::from(heard.split);
            pairs += u32::from(heard.completed_pair);
        }
        self.stats.send_twice_split_by_interloper += split;
        self.stats.send_twice_pairs_executed += pairs;
    }

    // IEC 62386-102 §11.7.14
    fn carry_prelude_over_held_half(&mut self, frame: u16, enabled: Option<u8>) {
        let Some(device_type) = enabled else { return };
        if self.gears.iter().any(|gear| gear.holds(frame)) {
            self.regs.enabled_device_type = Some((device_type, self.frame_seq));
        }
    }

    fn on_extended_dt8(
        &mut self,
        address: DaliAddress,
        opcode: u8,
        expects_backward: bool,
        enabled: Option<u8>,
    ) -> TransferOutcome {
        match enabled {
            Some(code) if code == DeviceType::Color.code() => {
                self.on_dt8(address, opcode, expects_backward)
            }
            Some(_) => self.publish_one(None),
            None => {
                self.stats.enable_device_type_consumed_by_interloper += 1;
                self.publish_one(None)
            }
        }
    }

    fn on_dapc(&mut self, address: DaliAddress, level: u8) -> TransferOutcome {
        for gear in self
            .gears
            .iter_mut()
            .filter(|g| g.matches(address) && g.executes_now)
        {
            // IEC 62386-102 §9.14.3.1
            gear.identifying = false;
            if level != DAPC_MASK_LEVEL {
                gear.arc_power_activation();
            }
            gear.apply_dapc(level);
        }
        self.publish_one(None)
    }

    fn on_special(&mut self, special: SpecialCommand) -> TransferOutcome {
        match special {
            SpecialCommand::Dtr0(v) => self.regs.dtr0 = v,
            SpecialCommand::Dtr1(v) => self.regs.dtr1 = v,
            SpecialCommand::Dtr2(v) => self.regs.dtr2 = v,
            SpecialCommand::EnableDeviceType(t) => {
                self.regs.enabled_device_type = Some((t, self.frame_seq));
            }
            SpecialCommand::SearchAddrH(v) => set_search_byte(&mut self.regs.search_address, 16, v),
            SpecialCommand::SearchAddrM(v) => set_search_byte(&mut self.regs.search_address, 8, v),
            SpecialCommand::SearchAddrL(v) => set_search_byte(&mut self.regs.search_address, 0, v),
            SpecialCommand::Terminate => self.terminate_initialise(),
            SpecialCommand::Initialise(scope) => self.enter_initialise(scope),
            SpecialCommand::Randomise => self.randomise(),
            SpecialCommand::Compare => return self.on_compare(),
            SpecialCommand::Withdraw => self.on_withdraw(),
            SpecialCommand::ProgramShortAddress(encoded) => self.program_short(encoded),
            SpecialCommand::VerifyShortAddress(encoded) => return self.verify_short(encoded),
            SpecialCommand::QueryShortAddress => return self.query_selected_short(),
            _ => {}
        }
        self.publish_one(None)
    }

    fn terminate_initialise(&mut self) {
        for gear in &mut self.gears {
            gear.initialise = false;
            gear.withdrawn = false;
        }
    }

    fn enter_initialise(&mut self, scope: u8) {
        for gear in self
            .gears
            .iter_mut()
            .filter(|g| g.enabled && g.executes_now)
        {
            let selected = match scope {
                0x00 => true,
                0xFF => gear.spec.short_address.is_none(),
                encoded => gear.spec.short_address == Some(decode_short_reply(encoded)),
            };
            if selected {
                gear.initialise = true;
                gear.withdrawn = false;
            }
        }
    }

    fn randomise(&mut self) {
        let rng = &mut self.rng;
        for gear in self
            .gears
            .iter_mut()
            .filter(|g| g.initialise && g.executes_now)
        {
            gear.spec.random_address = rng.next_24();
        }
    }

    fn on_compare(&mut self) -> TransferOutcome {
        let search = self.regs.search_address;
        let mut buf = self.take_scratch();
        for gear in &self.gears {
            if gear.enabled && gear.initialise && !gear.withdrawn && gear.spec.random_address <= search
            {
                buf.push(DALI_YES);
            }
        }
        self.publish(buf)
    }

    fn on_withdraw(&mut self) {
        let search = self.regs.search_address;
        for gear in &mut self.gears {
            if gear.initialise && gear.spec.random_address == search {
                gear.withdrawn = true;
            }
        }
    }

    fn program_short(&mut self, encoded: u8) {
        let search = self.regs.search_address;
        let short = decode_short_reply(encoded);
        if is_reserved(self.reserved, Some(short)) {
            self.stats.refused_programs += 1;
            return;
        }
        for gear in &mut self.gears {
            if gear.initialise && !gear.withdrawn && gear.spec.random_address == search {
                gear.spec.short_address = Some(short);
            }
        }
    }

    fn verify_short(&mut self, encoded: u8) -> TransferOutcome {
        let short = decode_short_reply(encoded);
        let mut buf = self.take_scratch();
        for gear in &self.gears {
            if gear.enabled && gear.initialise && gear.spec.short_address == Some(short) {
                buf.push(DALI_YES);
            }
        }
        self.publish(buf)
    }

    fn query_selected_short(&mut self) -> TransferOutcome {
        let search = self.regs.search_address;
        let mut buf = self.take_scratch();
        buf.extend(
            self.gears
                .iter()
                .filter(|g| g.enabled && g.initialise && !g.withdrawn)
                .filter(|g| g.spec.random_address == search)
                .filter_map(|g| g.spec.short_address)
                .map(encode_short_reply),
        );
        self.publish(buf)
    }

    fn on_read_memory_location(&mut self, address: DaliAddress) -> TransferOutcome {
        let (bank, offset) = (self.regs.dtr1, self.regs.dtr0);
        let mut buf = self.take_scratch();
        buf.extend(
            self.gears
                .iter()
                .filter(|g| g.matches(address))
                .filter_map(|g| g.memory_location(bank, offset)),
        );
        let bank_implemented = self
            .gears
            .iter()
            .any(|g| g.matches(address) && g.implements_bank(bank));
        // IEC 62386-102 §9.10.4
        if bank_implemented && offset < LAST_MEMORY_LOCATION {
            self.regs.dtr0 = offset + 1;
        }
        self.publish(buf)
    }

    fn on_dt8(
        &mut self,
        address: DaliAddress,
        opcode: u8,
        expects_backward: bool,
    ) -> TransferOutcome {
        if expects_backward {
            return self.on_dt8_query(address, opcode);
        }
        let (dtr0, dtr1, dtr2) = (self.regs.dtr0, self.regs.dtr1, self.regs.dtr2);
        let illegal_operand = opcode == Dt8Command::StoreGearFeaturesStatus.opcode()
            && dtr0 & GEAR_FEATURES_STORE_RESERVED_MASK != 0;
        let mut illegal_stores = 0u32;
        for gear in self
            .gears
            .iter_mut()
            .filter(|g| g.matches(address) && g.is_dt8() && g.executes_now)
        {
            apply_dt8_write(gear, opcode, dtr0, dtr1, dtr2);
            illegal_stores += u32::from(illegal_operand);
        }
        self.stats.dt8_gear_features_reserved_bits += illegal_stores;
        self.publish_one(None)
    }

    fn on_extended_dt6(
        &mut self,
        address: DaliAddress,
        opcode: u8,
        expects_backward: bool,
    ) -> TransferOutcome {
        if expects_backward {
            if opcode == QUERY_EXTENDED_VERSION_NUMBER_OPCODE {
                return self.constant_reply(address, EXTENDED_VERSION_NUMBER_207, Gear::is_dt6);
            }
            if let Some(outcome) = self.dt6_failure_query(address, opcode) {
                return outcome;
            }
            if opcode != Dt6Command::QueryDimmingCurve.opcode() {
                return self.publish_one(None);
            }
            let mut buf = self.take_scratch();
            buf.extend(
                self.gears
                    .iter()
                    .filter(|g| g.matches(address) && g.is_dt6())
                    .map(|g| g.dimming_curve),
            );
            return self.publish(buf);
        }
        if opcode == Dt6Command::SelectDimmingCurve.opcode() {
            let dtr0 = self.regs.dtr0;
            if dtr0 <= DIMMING_CURVE_MAX {
                for gear in self
                    .gears
                    .iter_mut()
                    .filter(|g| g.matches(address) && g.is_dt6() && g.executes_now)
                {
                    gear.dimming_curve = dtr0;
                }
            }
        }
        self.publish_one(None)
    }

    fn dt6_failure_query(&mut self, address: DaliAddress, opcode: u8) -> Option<TransferOutcome> {
        if opcode == Dt6Command::QueryFeatures.opcode() {
            return Some(self.constant_reply(address, DT6_FEATURES_QUERYABLE, Gear::is_dt6));
        }
        if opcode == Dt6Command::QueryFailureStatus.opcode() {
            let mut buf = self.take_scratch();
            buf.extend(
                self.gears
                    .iter()
                    .filter(|g| g.matches(address) && g.is_dt6())
                    .map(|g| g.faults.failure_status),
            );
            return Some(self.publish(buf));
        }
        let bit: fn(&Gear) -> bool = match Dt6Command::from_opcode(opcode)? {
            Dt6Command::QueryShortCircuit => |g| g.failure_bits().short_circuit,
            Dt6Command::QueryOpenCircuit => |g| g.failure_bits().open_circuit,
            Dt6Command::QueryLoadDecrease => |g| g.failure_bits().load_decrease,
            Dt6Command::QueryLoadIncrease => |g| g.failure_bits().load_increase,
            Dt6Command::QueryCurrentProtectorActive => {
                |g| g.failure_bits().current_protector_active
            }
            Dt6Command::QueryThermalShutdown => |g| g.failure_bits().thermal_shut_down,
            Dt6Command::QueryThermalOverload => |g| g.failure_bits().thermal_overload,
            Dt6Command::QueryReferenceMeasurementFailed => {
                |g| g.failure_bits().reference_measurement_failed
            }
            Dt6Command::QueryReferenceRunning | Dt6Command::QueryCurrentProtectorEnabled => {
                |_g| false
            }
            _ => return None,
        };
        let mut buf = self.take_scratch();
        buf.extend(
            self.gears
                .iter()
                .filter(|g| g.matches(address) && g.is_dt6() && bit(g))
                .map(|_| YES_ANSWER),
        );
        Some(self.publish(buf))
    }

    fn constant_reply(
        &mut self,
        address: DaliAddress,
        value: u8,
        eligible: fn(&Gear) -> bool,
    ) -> TransferOutcome {
        let mut buf = self.take_scratch();
        buf.extend(
            self.gears
                .iter()
                .filter(|g| g.matches(address) && eligible(g))
                .map(|_| value),
        );
        self.publish(buf)
    }

    fn on_dt8_query(&mut self, address: DaliAddress, opcode: u8) -> TransferOutcome {
        if opcode == Dt8Command::QueryColourValue.opcode() {
            return self.dt8_color_value_reply(address, self.regs.dtr0);
        }
        if opcode == QUERY_EXTENDED_VERSION_NUMBER_OPCODE {
            return self.constant_reply(address, EXTENDED_VERSION_NUMBER_209, Gear::is_dt8);
        }
        // IEC 62386-209 Table 14
        if opcode == Dt8Command::QueryRgbwafControl.opcode() {
            let mut buf = self.take_scratch();
            buf.extend(
                self.gears
                    .iter()
                    .filter(|g| g.matches(address) && g.is_dt8())
                    .filter_map(Gear::rgbwaf_control_byte),
            );
            return self.publish(buf);
        }
        let reply: fn(&Gear) -> u8 = if opcode == Dt8Command::QueryColourStatus.opcode() {
            Gear::dt8_colour_status
        } else if opcode == Dt8Command::QueryColourTypeFeatures.opcode() {
            Gear::dt8_features
        } else if opcode == Dt8Command::QueryGearFeaturesStatus.opcode() {
            Gear::gear_features_byte
        } else {
            return self.publish_one(None);
        };
        let mut buf = self.take_scratch();
        buf.extend(
            self.gears
                .iter()
                .filter(|g| g.matches(address) && g.is_dt8())
                .map(reply),
        );
        self.publish(buf)
    }

    fn dt8_color_value_reply(&mut self, address: DaliAddress, value_id: u8) -> TransferOutcome {
        let values: Vec<u16> = self
            .gears
            .iter()
            .filter(|g| g.matches(address) && g.is_dt8())
            .filter_map(|g| g.color_value(value_id))
            .collect();
        let wide = colour_value_is_wide(value_id);
        let mut buf = self.take_scratch();
        buf.extend(
            values
                .iter()
                .map(|v| if wide { (v >> 8) as u8 } else { *v as u8 }),
        );
        let lsb = values.first().map(|v| *v as u8);
        if values.iter().all(|v| Some(*v as u8) == lsb) {
            if let Some(lsb) = lsb {
                self.regs.dtr0 = lsb;
            }
        }
        self.publish(buf)
    }

    fn on_standard(&mut self, address: DaliAddress, command: StandardCommand) -> TransferOutcome {
        if let Some(reply) = self.standard_query(address, command) {
            return reply;
        }
        let dtr0 = self.regs.dtr0;
        let reserved = self.reserved;
        let mut refused_programs = 0u32;
        let mut store_dtr0 = None;
        for gear in self
            .gears
            .iter_mut()
            .filter(|g| g.matches(address) && g.executes_now)
        {
            apply_standard_write(gear, command, dtr0);
            if matches!(command, StandardCommand::SetShortAddress) {
                match set_short_from_dtr0(gear, dtr0, reserved) {
                    ShortAddressWrite::Refused => refused_programs += 1,
                    ShortAddressWrite::Ignored | ShortAddressWrite::Applied => {}
                }
            }
            if matches!(command, StandardCommand::StoreActualLevelInDtr0) {
                store_dtr0 = Some(gear.level);
                // IEC 62386-209 §9.12.6
                if gear.is_dt8() {
                    gear.load_report_from_actual();
                }
            }
        }
        if let Some(level) = store_dtr0 {
            self.regs.dtr0 = level;
        }
        self.stats.refused_programs += refused_programs;
        self.publish_one(None)
    }

    fn standard_query(
        &mut self,
        address: DaliAddress,
        command: StandardCommand,
    ) -> Option<TransferOutcome> {
        if !command.is_query() {
            return None;
        }
        let shared_register = self.bus_register_query(command);
        let mut buf = self.take_scratch();
        let (gears, rng, stats) = (&mut self.gears, &mut self.rng, &mut self.stats);
        buf.extend(
            gears
                .iter_mut()
                .filter(|g| g.matches(address))
                .filter_map(|g| {
                    let reply = match shared_register {
                        Some(value) => TransferOutcome::Answer(value),
                        None => gear_query_reply(g, command),
                    };
                    answer_of(reply, g, rng, stats)
                }),
        );
        Some(self.publish(buf))
    }

    fn bus_register_query(&self, command: StandardCommand) -> Option<u8> {
        match command {
            StandardCommand::QueryContentDtr0 => Some(self.regs.dtr0),
            StandardCommand::QueryContentDtr1 => Some(self.regs.dtr1),
            StandardCommand::QueryContentDtr2 => Some(self.regs.dtr2),
            _ => None,
        }
    }
}

fn answer_of(
    outcome: TransferOutcome,
    gear: &Gear,
    rng: &mut Rng,
    stats: &mut FleetStats,
) -> Option<u8> {
    let TransferOutcome::Answer(value) = outcome else {
        return None;
    };
    if rng.chance_permille(gear.faults.drop_answer_permille) {
        stats.dropped_answers += 1;
        return None;
    }
    Some(value)
}

// IEC 62386-101 §8.2.5
fn merge_answers(answers: &[u8]) -> TransferOutcome {
    match answers {
        [] => TransferOutcome::NoAnswer,
        [only] => TransferOutcome::Answer(*only),
        _ => TransferOutcome::CorruptedInWindow,
    }
}

fn is_reserved(reserved: u64, short: Option<u8>) -> bool {
    match short {
        Some(addr) if addr < 64 => reserved & (1u64 << addr) != 0,
        _ => false,
    }
}

pub(crate) fn demo_random_address(index: u8) -> u32 {
    const DEMO_RANDOM_STRIDE: u32 = 0x0021_4365;
    (u32::from(index) + 1).wrapping_mul(DEMO_RANDOM_STRIDE) & 0x00FF_FFFF
}

const DEMO_RNG_SEED: u32 = 0x00C0_FFEE;

fn set_search_byte(search: &mut u32, shift: u32, value: u8) {
    *search = (*search & !(0xFFu32 << shift)) | (u32::from(value) << shift);
}

pub(crate) fn encode_short_reply(short: u8) -> u8 {
    ((short & SHORT_ADDRESS_MASK) << 1) | 0x01
}

const DALI_MASK_BYTE: u8 = 0xFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortAddressWrite {
    Applied,
    Refused,
    Ignored,
}

// IEC 62386-102 §11.4.19
fn set_short_from_dtr0(gear: &mut Gear, dtr0: u8, reserved: u64) -> ShortAddressWrite {
    if dtr0 == DALI_MASK_BYTE {
        gear.spec.short_address = None;
        return ShortAddressWrite::Applied;
    }
    if dtr0 & 0x81 != 0x01 {
        return ShortAddressWrite::Ignored;
    }
    let short = decode_short_reply(dtr0);
    if is_reserved(reserved, Some(short)) {
        return ShortAddressWrite::Refused;
    }
    gear.spec.short_address = Some(short);
    ShortAddressWrite::Applied
}

fn decode_short_reply(encoded: u8) -> u8 {
    (encoded >> 1) & SHORT_ADDRESS_MASK
}

#[derive(Debug, Clone, Copy)]
enum Audience {
    // IEC 62386-102 §12.3.15
    Every,
    Addressed(DaliAddress),
}

fn is_walk_query(decoded: Option<&DaliCommand>) -> bool {
    matches!(
        decoded,
        Some(DaliCommand::Standard {
            command: StandardCommand::QueryDeviceType | StandardCommand::QueryNextDeviceType,
            ..
        })
    )
}

fn audience_of(decoded: Option<&DaliCommand>) -> Option<Audience> {
    match decoded? {
        DaliCommand::Special(SpecialCommand::Ping) => None,
        DaliCommand::Special(_) => Some(Audience::Every),
        DaliCommand::Standard { address, .. } | DaliCommand::Extended { address, .. } => {
            Some(Audience::Addressed(*address))
        }
        _ => None,
    }
}

fn requires_repeat_under(
    decoded: Option<&DaliCommand>,
    wire_address: u8,
    opcode: u8,
    device_type: Option<u8>,
) -> bool {
    let indirect = wire_address & 0x01 != 0;
    if indirect && device_type == Some(DeviceType::Color.code()) {
        if let Some(command) = Dt8Command::from_opcode(opcode) {
            return ExtendedCommand::Dt8(command).requires_repeat();
        }
    }
    if indirect && device_type == Some(DeviceType::Led.code()) {
        if let Some(command) = Dt6Command::from_opcode(opcode) {
            return ExtendedCommand::Dt6(command).requires_repeat();
        }
    }
    decoded.is_some_and(DaliCommand::requires_repeat)
}

fn is_dt8_opcode(opcode: u8) -> bool {
    Dt8Command::from_opcode(opcode).is_some()
}

fn is_dt6_opcode(opcode: u8) -> bool {
    Dt6Command::from_opcode(opcode).is_some()
}

const DIMMING_CURVE_MAX: u8 = 1;

const QUERY_EXTENDED_VERSION_NUMBER_OPCODE: u8 = 0xFF;

// IEC 62386-207 §11.3.4.2
const EXTENDED_VERSION_NUMBER_207: u8 = 1;

const DT6_FEATURES_QUERYABLE: u8 = 0x7F;

const YES_ANSWER: u8 = 0xFF;

// IEC 62386-209 §11.3.4.3
const EXTENDED_VERSION_NUMBER_209: u8 = 2;

const LAST_MEMORY_LOCATION: u8 = 0xFF;

fn is_dt8_query_opcode(opcode: u8) -> bool {
    matches!(
        Dt8Command::from_opcode(opcode),
        Some(
            Dt8Command::QueryGearFeaturesStatus
                | Dt8Command::QueryColourStatus
                | Dt8Command::QueryColourTypeFeatures
                | Dt8Command::QueryColourValue
                | Dt8Command::QueryRgbwafControl
        )
    )
}

fn apply_dt8_write(gear: &mut Gear, opcode: u8, dtr0: u8, dtr1: u8, dtr2: u8) {
    let word = (u16::from(dtr1) << 8) | u16::from(dtr0);
    match Dt8Command::from_opcode(opcode) {
        Some(Dt8Command::SetTemporaryXCoordinate) => {
            gear.pending_x = word;
            gear.pending_mode = crate::gear::ColorMode::Xy;
        }
        Some(Dt8Command::SetTemporaryYCoordinate) => {
            gear.pending_y = word;
            gear.pending_mode = crate::gear::ColorMode::Xy;
        }
        Some(Dt8Command::SetTemporaryColourTemperature) => {
            gear.pending_ct = word;
            gear.pending_mode = crate::gear::ColorMode::Cct;
        }
        Some(Dt8Command::SetTemporaryRgbDimLevel) => {
            gear.rgbwaf.pending_levels[..3].copy_from_slice(&[dtr0, dtr1, dtr2]);
            gear.pending_mode = crate::gear::ColorMode::Rgb;
        }
        Some(Dt8Command::SetTemporaryWafDimLevel) => {
            gear.rgbwaf.pending_levels[3..].copy_from_slice(&[dtr0, dtr1, dtr2]);
            gear.pending_mode = crate::gear::ColorMode::Rgb;
        }
        // IEC 62386-209 §9.12.4
        Some(Dt8Command::SetTemporaryRgbwafControl) => {
            gear.rgbwaf.pending_control = dtr0;
            gear.pending_mode = crate::gear::ColorMode::Rgb;
        }
        Some(Dt8Command::Activate) => gear.activate_pending_color(),
        Some(Dt8Command::XCoordinateStepUp) => gear.xy_step(false, true),
        Some(Dt8Command::XCoordinateStepDown) => gear.xy_step(false, false),
        Some(Dt8Command::YCoordinateStepUp) => gear.xy_step(true, true),
        Some(Dt8Command::YCoordinateStepDown) => gear.xy_step(true, false),
        Some(Dt8Command::ColourTemperatureStepCooler) => gear.tc_step(false),
        Some(Dt8Command::ColourTemperatureStepWarmer) => gear.tc_step(true),
        Some(Dt8Command::CopyReportToTemporary) => gear.copy_report_to_temporary(),
        Some(Dt8Command::StoreColourTemperatureTcLimit) => {
            gear.store_tc_limit(word, dtr2);
        }
        Some(Dt8Command::StoreGearFeaturesStatus) => store_gear_features(gear, dtr0),
        _ => {}
    }
}

// IEC 62386-209 §11.3.4.2, Table 8
fn store_gear_features(gear: &mut Gear, dtr0: u8) {
    let kept = gear.gear_features & !GEAR_FEATURES_AUTOMATIC_ACTIVATION;
    gear.gear_features = kept | (dtr0 & GEAR_FEATURES_AUTOMATIC_ACTIVATION);
}

const SHORT_ADDRESS_MAX: u8 = SHORT_ADDRESS_MASK;

pub fn bench_fleet(base: u8, dt6: u8, cct: u8, rgb: u8, seed: u32) -> Vec<GearSpec> {
    let mut rng = Rng::new(seed);
    let mut specs = Vec::new();
    let mut short = base;
    let push = |kind: u8, short: Option<u8>, rng: &mut Rng| match kind {
        0 => GearSpec::dt6(short, rng.next_24()),
        1 => GearSpec::dt8(
            short,
            rng.next_24(),
            (true, false, false),
            (TC_COOLEST_MIREK, TC_WARMEST_MIREK),
        ),
        _ => GearSpec::dt8(
            short,
            rng.next_24(),
            (true, true, true),
            (TC_COOLEST_MIREK, TC_WARMEST_MIREK),
        ),
    };
    for (kind, count) in [(0u8, dt6), (1, cct), (2, rgb)] {
        for _ in 0..count {
            while short <= SHORT_ADDRESS_MAX
                && is_reserved(DEFAULT_RESERVED_SHORT_ADDRESSES, Some(short))
            {
                short += 1;
            }
            specs.push(push(kind, (short <= SHORT_ADDRESS_MAX).then_some(short), &mut rng));
            short = short.saturating_add(1);
        }
    }
    specs
}
