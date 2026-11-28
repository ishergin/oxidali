use serde::{Deserialize, Serialize};

use super::bounded::FixedText64;
use super::kinds::LevelTransition;
use super::state::LightSetpoint;

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
    RecordName,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterBirthMark {
    pub generation: u32,
    pub nonce: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterRecordRef {
    pub number: u8,
    pub mark: ClusterBirthMark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterPartLink {
    pub record: ClusterRecordRef,
    pub send: bool,
    pub accept: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClusterRecordKind {
    Channel,
    Scene,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterRecordAction {
    Create { number: Option<u8>, nonce: u32, node: u8 },
    Rename { number: u8, revision: u32 },
    Delete { number: u8, revision: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterLampAddress {
    pub node: u8,
    pub adapter: u8,
    pub lamp: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterTarget {
    Channel(ClusterRecordRef),
    All,
    Lamp(ClusterLampAddress),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterRecallScope {
    Node,
    Channel(ClusterRecordRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterLightVerb {
    Setpoint(LightSetpoint),
    Transition(LevelTransition),
    StopFade,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterAction {
    Light { target: ClusterTarget, verb: ClusterLightVerb },
    RecallScene { scene: ClusterRecordRef, scope: ClusterRecallScope },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterSourceClass {
    Operator,
    HomeAssistant,
    Hcl,
    Rules,
    ForeignMaster,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterSubject {
    Status,
    Info,
    Lamps,
    Inputs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterCopyAction {
    Store { revision: u32, name: FixedText64 },
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterTakenRecord {
    pub mark: ClusterBirthMark,
    pub revision: u32,
    pub name: FixedText64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_field_order_is_the_wire_discriminant() {
        assert_eq!(postcard::to_allocvec(&ClusterTextField::ClusterName).expect("encodes"), [0]);
    }

    #[test]
    fn the_record_kinds_and_actions_keep_their_wire_discriminants() {
        let encode = |value: &dyn erased::Encode| value.encode();
        assert_eq!(encode(&ClusterTextField::RecordName), [1]);
        assert_eq!(encode(&ClusterRecordKind::Channel), [0]);
        assert_eq!(encode(&ClusterRecordKind::Scene), [1]);
        assert_eq!(encode(&ClusterRecordAction::Create { number: None, nonce: 0, node: 2 }), [0, 0, 0, 2]);
        assert_eq!(encode(&ClusterRecordAction::Rename { number: 3, revision: 2 }), [1, 3, 2]);
        assert_eq!(encode(&ClusterRecordAction::Delete { number: 3, revision: 2 }), [2, 3, 2]);
    }

    #[test]
    fn the_birth_mark_is_the_generation_then_the_nonce() {
        let mark = ClusterBirthMark { generation: 1, nonce: 2 };
        assert_eq!(postcard::to_allocvec(&mark).expect("encodes"), [1, 2]);
    }

    #[test]
    fn a_part_link_is_the_record_then_send_then_accept() {
        let record = ClusterRecordRef { number: 20, mark: ClusterBirthMark { generation: 1, nonce: 2 } };
        let link = ClusterPartLink { record, send: true, accept: false };
        assert_eq!(postcard::to_allocvec(&link).expect("encodes"), [20, 1, 2, 1, 0]);
    }

    mod erased {
        pub trait Encode {
            fn encode(&self) -> Vec<u8>;
        }

        impl<T: serde::Serialize> Encode for T {
            fn encode(&self) -> Vec<u8> {
                postcard::to_allocvec(self).expect("encodes")
            }
        }
    }

    #[test]
    fn the_hcl_tick_that_came_over_the_cluster_is_appended_to_both_origin_tables() {
        let origin = postcard::to_allocvec(&crate::msg::Origin::ClusterHcl).expect("encodes");
        let source = postcard::to_allocvec(&crate::msg::RuntimeSource::ClusterHcl).expect("encodes");
        assert_eq!((origin, source), (vec![10], vec![9]));
        let provenance = crate::msg::RuntimeSource::from_origin(crate::msg::Origin::ClusterHcl);
        assert_eq!(provenance, Some(crate::msg::RuntimeSource::ClusterHcl));
        assert!(crate::msg::RuntimeSource::ClusterHcl.is_hcl());
    }

    #[test]
    fn the_forwarding_fields_keep_their_wire_order() {
        let only_hcl = ClusterForwarding { hcl: true, ..ClusterForwarding::default() };
        assert_eq!(postcard::to_allocvec(&only_hcl).expect("encodes"), [0, 0, 1, 0]);
    }
}
