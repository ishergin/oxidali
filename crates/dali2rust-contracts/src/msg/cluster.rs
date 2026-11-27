use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterForwarding {
    pub operator: bool,
    pub home_assistant: bool,
    pub hcl: bool,
    pub foreign_master: bool,
}

impl ClusterForwarding {
    pub const FACTORY: Self = Self {
        operator: true,
        home_assistant: true,
        hcl: false,
        foreign_master: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterTextField {
    ClusterName,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_field_order_is_the_wire_discriminant() {
        assert_eq!(postcard::to_allocvec(&ClusterTextField::ClusterName).expect("encodes"), [0]);
    }

    #[test]
    fn the_forwarding_fields_keep_their_wire_order() {
        let only_hcl = ClusterForwarding { hcl: true, ..ClusterForwarding::default() };
        assert_eq!(postcard::to_allocvec(&only_hcl).expect("encodes"), [0, 0, 1, 0]);
    }
}
