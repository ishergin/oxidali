use dali2rust_api::contracts::dali::wire_address_from_short;
use dali2rust_domain::dali::types::DaliAddress;

#[test]
fn short_addresses_encode_to_wire_bytes() {
    let cases = [(0u8, 0u8), (1, 2), (63, 126)];
    for (short, expected) in cases {
        assert_eq!(
            wire_address_from_short(short).expect("short in range"),
            expected
        );
    }
}

#[test]
fn broadcast_uses_canonical_wire_byte() {
    assert_eq!(DaliAddress::Broadcast.encode_address_byte(), 254);
}

#[test]
fn group_addresses_encode_via_domain_net_layer() {
    let cases = [(0u8, 128u8), (1, 130), (15, 158)];
    for (group, expected) in cases {
        let address = DaliAddress::group(group).expect("group in range");
        assert_eq!(address.encode_address_byte(), expected);
    }
}
