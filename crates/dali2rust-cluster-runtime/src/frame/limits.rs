use dali2rust_domain::registry::{
    CLUSTER_NODE_COUNT, CLUSTER_NUMBERS_PER_NODE, CLUSTER_RECORD_NUMBER_COUNT, GROUP_COUNT,
    MAX_ADAPTERS, MAX_CLUSTER_NAME_BYTES as DOMAIN_CLUSTER_NAME_BYTES, SCENE_COUNT, VIRTUAL_LAMP_COUNT,
};

pub const NODE_COUNT: u8 = CLUSTER_NODE_COUNT;

pub const NUMBERS_PER_NODE: u8 = CLUSTER_NUMBERS_PER_NODE;

pub const RECORD_NUMBER_COUNT: u8 = CLUSTER_RECORD_NUMBER_COUNT;

pub const MAX_NAME_BYTES: usize = 64;

pub const MAX_CLUSTER_NAME_BYTES: usize = DOMAIN_CLUSTER_NAME_BYTES;

pub const ADAPTER_COUNT: u8 = MAX_ADAPTERS;

pub const MAX_INSTALLATION_BYTES: usize = 32;

pub(crate) const fn is_node(node: u8) -> bool {
    node < NODE_COUNT
}

pub(crate) const fn is_adapter(adapter: u8) -> bool {
    adapter < ADAPTER_COUNT
}

pub(crate) const fn is_record_number(number: u8) -> bool {
    number < RECORD_NUMBER_COUNT
}

pub(crate) const fn is_lamp(lamp: u8) -> bool {
    lamp < VIRTUAL_LAMP_COUNT
}

pub(crate) const fn is_group(group: u8) -> bool {
    group < GROUP_COUNT
}

pub(crate) const fn is_node_scene(scene: u8) -> bool {
    scene < SCENE_COUNT
}

#[cfg(test)]
mod tests {
    use dali2rust_contracts::msg::{FixedText32, FixedText64};

    use super::*;

    #[test]
    fn a_record_name_holds_what_a_registry_name_holds() {
        assert_eq!(FixedText64::new().capacity(), MAX_NAME_BYTES);
    }

    #[test]
    fn the_installation_holds_what_the_controller_id_holds() {
        assert_eq!(FixedText32::new().capacity(), MAX_INSTALLATION_BYTES);
    }

    #[test]
    fn every_node_has_a_block_of_sixteen_numbers_and_eight_blocks_fill_the_numbers() {
        assert_eq!(RECORD_NUMBER_COUNT, 128);
        assert!(is_record_number(RECORD_NUMBER_COUNT - 1));
        assert!(!is_record_number(RECORD_NUMBER_COUNT));
    }
}
