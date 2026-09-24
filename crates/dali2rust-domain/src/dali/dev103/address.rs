#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device103Address {
    Short(u8),
    Group(u8),
    Broadcast,
    BroadcastUnaddressed,
    Special(u8),
}

pub const MAX_SHORT_ADDRESS: u8 = 63;
pub const MAX_DEVICE_GROUP: u8 = 31;
pub const MAX_SPECIAL_SELECTOR: u8 = 15;

const BROADCAST_BYTE: u8 = 0xFF;
const BROADCAST_UNADDRESSED_BYTE: u8 = 0xFD;
const COMMAND_BIT: u8 = 0x01;

impl Device103Address {
    #[must_use]
    pub const fn encode(self) -> u8 {
        match self {
            Self::Short(a) => (a << 1) | COMMAND_BIT,
            Self::Group(g) => 0x80 | (g << 1) | COMMAND_BIT,
            Self::Broadcast => BROADCAST_BYTE,
            Self::BroadcastUnaddressed => BROADCAST_UNADDRESSED_BYTE,
            Self::Special(s) => 0xC0 | (s << 1) | COMMAND_BIT,
        }
    }

    #[must_use]
    pub const fn decode(byte: u8) -> Option<Self> {
        match byte | COMMAND_BIT {
            BROADCAST_BYTE => Some(Self::Broadcast),
            BROADCAST_UNADDRESSED_BYTE => Some(Self::BroadcastUnaddressed),
            b if b & 0x80 == 0 => Some(Self::Short((b >> 1) & 0x3F)),
            b if b & 0xC0 == 0x80 => Some(Self::Group((b >> 1) & 0x1F)),
            b if b & 0xE0 == 0xC0 => Some(Self::Special((b >> 1) & 0x0F)),
            _ => None,
        }
    }

    #[must_use]
    pub const fn is_unicast(self) -> bool {
        matches!(self, Self::Short(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceAddress {
    Number(u8),
    Group(u8),
    Type(u8),
    FeatureNumber(u8),
    FeatureGroup(u8),
    FeatureType(u8),
    FeatureBroadcast,
    FeatureDevice,
    Device,
    Broadcast,
}

pub const MAX_INSTANCE_INDEX: u8 = 31;

const INSTANCE_DEVICE_BYTE: u8 = 0xFE;
const INSTANCE_BROADCAST_BYTE: u8 = 0xFF;
const INSTANCE_FEATURE_BROADCAST_BYTE: u8 = 0xFD;
const INSTANCE_FEATURE_DEVICE_BYTE: u8 = 0xFC;

impl InstanceAddress {
    #[must_use]
    pub const fn encode(self) -> u8 {
        match self {
            Self::Number(n) => n & 0x1F,
            Self::Group(g) => 0x80 | (g & 0x1F),
            Self::Type(t) => 0xC0 | (t & 0x1F),
            Self::FeatureNumber(n) => 0x20 | (n & 0x1F),
            Self::FeatureGroup(g) => 0xA0 | (g & 0x1F),
            Self::FeatureType(t) => 0x60 | (t & 0x1F),
            Self::FeatureBroadcast => INSTANCE_FEATURE_BROADCAST_BYTE,
            Self::FeatureDevice => INSTANCE_FEATURE_DEVICE_BYTE,
            Self::Device => INSTANCE_DEVICE_BYTE,
            Self::Broadcast => INSTANCE_BROADCAST_BYTE,
        }
    }

    #[must_use]
    pub const fn decode(byte: u8) -> Option<Self> {
        match byte {
            INSTANCE_DEVICE_BYTE => Some(Self::Device),
            INSTANCE_BROADCAST_BYTE => Some(Self::Broadcast),
            INSTANCE_FEATURE_BROADCAST_BYTE => Some(Self::FeatureBroadcast),
            INSTANCE_FEATURE_DEVICE_BYTE => Some(Self::FeatureDevice),
            b => {
                let value = b & 0x1F;
                match b & 0xE0 {
                    0x00 => Some(Self::Number(value)),
                    0x80 => Some(Self::Group(value)),
                    0xC0 => Some(Self::Type(value)),
                    0x20 => Some(Self::FeatureNumber(value)),
                    0xA0 => Some(Self::FeatureGroup(value)),
                    0x60 => Some(Self::FeatureType(value)),
                    _ => None,
                }
            }
        }
    }

    #[must_use]
    pub const fn is_unicast(self) -> bool {
        matches!(self, Self::Number(_))
    }
}

// IEC 62386-103 §11.10.10
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortAddressOperand(u8);

pub const SHORT_ADDRESS_MASK: u8 = 0xFF;

impl ShortAddressOperand {
    #[must_use]
    pub const fn new(address: u8) -> Option<Self> {
        if address <= MAX_SHORT_ADDRESS {
            Some(Self(address))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn mask() -> Self {
        Self(SHORT_ADDRESS_MASK)
    }

    #[must_use]
    pub const fn as_byte(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn address(self) -> Option<u8> {
        if self.0 <= MAX_SHORT_ADDRESS {
            Some(self.0)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn from_answer(byte: u8) -> Self {
        Self(byte)
    }
}

// IEC 62386-103 Table 23
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialiseScope103 {
    Device(u8),
    Unaddressed,
    All,
}

const INITIALISE_UNADDRESSED: u8 = 0x7F;
const INITIALISE_ALL: u8 = 0xFF;

impl InitialiseScope103 {
    #[must_use]
    pub const fn encode(self) -> u8 {
        match self {
            Self::Device(a) => a & 0x3F,
            Self::Unaddressed => INITIALISE_UNADDRESSED,
            Self::All => INITIALISE_ALL,
        }
    }

    #[must_use]
    pub const fn decode(byte: u8) -> Option<Self> {
        match byte {
            INITIALISE_UNADDRESSED => Some(Self::Unaddressed),
            INITIALISE_ALL => Some(Self::All),
            b if b <= MAX_SHORT_ADDRESS => Some(Self::Device(b)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_address_round_trips_over_the_whole_space() {
        for a in 0..=MAX_SHORT_ADDRESS {
            let byte = Device103Address::Short(a).encode();
            assert_eq!(byte & COMMAND_BIT, COMMAND_BIT, "command bit must be set");
            assert_eq!(Device103Address::decode(byte), Some(Device103Address::Short(a)));
            assert_eq!(
                Device103Address::decode(byte & !COMMAND_BIT),
                Some(Device103Address::Short(a))
            );
        }
    }

    #[test]
    fn device_groups_round_trip_and_there_are_thirty_two_of_them() {
        for g in 0..=MAX_DEVICE_GROUP {
            let byte = Device103Address::Group(g).encode();
            assert_eq!(Device103Address::decode(byte), Some(Device103Address::Group(g)));
        }
        assert_eq!(
            Device103Address::decode(Device103Address::Group(31).encode()),
            Some(Device103Address::Group(31))
        );
    }

    #[test]
    fn broadcast_forms_are_distinct_and_not_read_as_special() {
        assert_eq!(Device103Address::decode(0xFF), Some(Device103Address::Broadcast));
        assert_eq!(
            Device103Address::decode(0xFD),
            Some(Device103Address::BroadcastUnaddressed)
        );
        assert_eq!(Device103Address::Broadcast.encode(), 0xFF);
        assert_eq!(Device103Address::BroadcastUnaddressed.encode(), 0xFD);
    }

    #[test]
    fn special_selectors_occupy_c1_to_df() {
        assert_eq!(Device103Address::Special(0).encode(), 0xC1);
        assert_eq!(Device103Address::Special(15).encode(), 0xDF);
        for s in 0..=MAX_SPECIAL_SELECTOR {
            let byte = Device103Address::Special(s).encode();
            assert_eq!(Device103Address::decode(byte), Some(Device103Address::Special(s)));
        }
    }

    #[test]
    fn reserved_address_patterns_decode_to_nothing() {
        for byte in [0xE1u8, 0xE3, 0xEF, 0xF1, 0xF7, 0xF9, 0xFB] {
            assert_eq!(Device103Address::decode(byte), None, "0x{byte:02X}");
        }
    }

    #[test]
    fn every_instance_selector_round_trips() {
        let mut cases: Vec<InstanceAddress> = Vec::new();
        for i in 0..=MAX_INSTANCE_INDEX {
            cases.extend([
                InstanceAddress::Number(i),
                InstanceAddress::Group(i),
                InstanceAddress::Type(i),
                InstanceAddress::FeatureNumber(i),
                InstanceAddress::FeatureGroup(i),
                InstanceAddress::FeatureType(i),
            ]);
        }
        cases.extend([
            InstanceAddress::FeatureBroadcast,
            InstanceAddress::FeatureDevice,
            InstanceAddress::Device,
            InstanceAddress::Broadcast,
        ]);
        for case in cases {
            assert_eq!(InstanceAddress::decode(case.encode()), Some(case), "{case:?}");
        }
    }

    #[test]
    fn the_device_selector_is_0xfe_because_every_device_command_depends_on_it() {
        assert_eq!(InstanceAddress::Device.encode(), 0xFE);
    }

    #[test]
    fn reserved_instance_patterns_decode_to_nothing() {
        for byte in [0x40u8, 0x5F, 0xE0, 0xEF, 0xF0, 0xFB] {
            assert_eq!(InstanceAddress::decode(byte), None, "0x{byte:02X}");
        }
    }

    #[test]
    fn short_address_operand_is_raw_not_the_102_form() {
        assert_eq!(ShortAddressOperand::new(3).unwrap().as_byte(), 0b0000_0011);
        assert_eq!(ShortAddressOperand::new(63).unwrap().as_byte(), 0b0011_1111);
        assert_eq!(ShortAddressOperand::new(64), None);
        assert_eq!(ShortAddressOperand::mask().as_byte(), 0xFF);
        assert_eq!(ShortAddressOperand::mask().address(), None);
    }

    #[test]
    fn no_change_operands_are_not_addresses() {
        for byte in [0x40u8, 0x80, 0xFE] {
            assert_eq!(ShortAddressOperand::from_answer(byte).address(), None, "0x{byte:02X}");
        }
    }

    #[test]
    fn initialise_scope_is_inverted_relative_to_part_102() {
        assert_eq!(InitialiseScope103::Unaddressed.encode(), 0x7F);
        assert_eq!(InitialiseScope103::All.encode(), 0xFF);
        assert_eq!(InitialiseScope103::Device(5).encode(), 0x05);
        assert_eq!(InitialiseScope103::decode(0x7F), Some(InitialiseScope103::Unaddressed));
        assert_eq!(InitialiseScope103::decode(0xFF), Some(InitialiseScope103::All));
        assert_eq!(InitialiseScope103::decode(0x05), Some(InitialiseScope103::Device(5)));
        assert_eq!(InitialiseScope103::decode(0x00), Some(InitialiseScope103::Device(0)));
        assert_eq!(InitialiseScope103::decode(0x80), None);
    }
}
