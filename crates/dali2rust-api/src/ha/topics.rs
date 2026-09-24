pub fn is_topic_safe(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HaTopics {
    pub discovery_prefix: String,
    pub state_prefix: String,
    pub controller_id: String,
    command_root: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HaCommandTarget {
    VirtualLamp { adapter_id: u8, virtual_lamp_id: u8 },
    Group { adapter_id: u8, group_id: u8 },
    SceneSelect { adapter_id: u8 },
}

impl HaTopics {
    pub fn new(discovery_prefix: &str, state_prefix: &str, controller_id: &str) -> Self {
        Self {
            discovery_prefix: discovery_prefix.to_string(),
            state_prefix: state_prefix.to_string(),
            controller_id: controller_id.to_string(),
            command_root: format!("{state_prefix}/{controller_id}/"),
        }
    }

    pub fn lamp_object_id(adapter_id: u8, virtual_lamp_id: u8) -> String {
        format!("a{adapter_id}_vl_{virtual_lamp_id}")
    }

    pub fn group_object_id(adapter_id: u8, group_id: u8) -> String {
        format!("a{adapter_id}_group_{group_id}")
    }

    pub fn input_object_id(adapter_id: u8, short_address: u8, instance_number: u8) -> String {
        format!("a{adapter_id}_in{short_address}_{instance_number}")
    }

    pub fn scene_select_object_id(adapter_id: u8) -> String {
        format!("a{adapter_id}_scene_select")
    }

    pub fn unique_id(&self, object_id: &str) -> String {
        format!("{}_{object_id}", self.controller_id)
    }

    pub fn discovery_topic(&self, component: &str, object_id: &str) -> String {
        format!(
            "{}/{component}/{}/{object_id}/config",
            self.discovery_prefix, self.controller_id
        )
    }

    pub fn input_state_topic(&self, adapter_id: u8, short_address: u8, instance_number: u8) -> String {
        format!(
            "{}/{}/a{adapter_id}/in/{short_address}/{instance_number}/state",
            self.state_prefix, self.controller_id
        )
    }

    pub fn lamp_state_topic(&self, adapter_id: u8, virtual_lamp_id: u8) -> String {
        format!(
            "{}/{}/a{adapter_id}/vl/{virtual_lamp_id}/state",
            self.state_prefix, self.controller_id
        )
    }

    pub fn lamp_command_topic(&self, adapter_id: u8, virtual_lamp_id: u8) -> String {
        format!(
            "{}/{}/a{adapter_id}/vl/{virtual_lamp_id}/set",
            self.state_prefix, self.controller_id
        )
    }

    pub fn group_state_topic(&self, adapter_id: u8, group_id: u8) -> String {
        format!(
            "{}/{}/a{adapter_id}/group/{group_id}/state",
            self.state_prefix, self.controller_id
        )
    }

    pub fn group_command_topic(&self, adapter_id: u8, group_id: u8) -> String {
        format!(
            "{}/{}/a{adapter_id}/group/{group_id}/set",
            self.state_prefix, self.controller_id
        )
    }

    pub fn scene_select_state_topic(&self, adapter_id: u8) -> String {
        format!(
            "{}/{}/a{adapter_id}/scene_select/state",
            self.state_prefix, self.controller_id
        )
    }

    pub fn scene_select_command_topic(&self, adapter_id: u8) -> String {
        format!(
            "{}/{}/a{adapter_id}/scene_select/set",
            self.state_prefix, self.controller_id
        )
    }

    pub fn availability_topic(&self) -> String {
        format!("{}/{}/availability", self.state_prefix, self.controller_id)
    }

    pub fn lamp_availability_topic(&self, adapter_id: u8, virtual_lamp_id: u8) -> String {
        format!(
            "{}/{}/a{adapter_id}/vl/{virtual_lamp_id}/availability",
            self.state_prefix, self.controller_id
        )
    }

    pub fn command_subscriptions(&self) -> [String; 3] {
        let root = format!("{}/{}", self.state_prefix, self.controller_id);
        [
            format!("{root}/+/vl/+/set"),
            format!("{root}/+/group/+/set"),
            format!("{root}/+/scene_select/set"),
        ]
    }

    pub fn parse_command_topic(&self, topic: &str) -> Option<HaCommandTarget> {
        let rest = topic.strip_prefix(&self.command_root)?;
        let parts: Vec<&str> = rest.split('/').collect();
        let adapter_id = parts.first()?.strip_prefix('a')?.parse().ok()?;
        match parts[1..] {
            ["vl", id, "set"] => Some(HaCommandTarget::VirtualLamp {
                adapter_id,
                virtual_lamp_id: id.parse().ok()?,
            }),
            ["group", id, "set"] => Some(HaCommandTarget::Group {
                adapter_id,
                group_id: id.parse().ok()?,
            }),
            ["scene_select", "set"] => Some(HaCommandTarget::SceneSelect { adapter_id }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topics() -> HaTopics {
        HaTopics::new("homeassistant", "dali", "ctl1")
    }

    #[test]
    fn unique_ids_match_the_documented_formulas() {
        let t = topics();
        assert_eq!(t.unique_id(&HaTopics::lamp_object_id(0, 12)), "ctl1_a0_vl_12");
        assert_eq!(t.unique_id(&HaTopics::group_object_id(0, 7)), "ctl1_a0_group_7");
        assert_eq!(
            t.unique_id(&HaTopics::scene_select_object_id(0)),
            "ctl1_a0_scene_select"
        );
    }

    #[test]
    fn a_config_topic_carries_the_same_object_id_as_the_unique_id() {
        let t = topics();
        let object_id = HaTopics::lamp_object_id(0, 12);
        assert_eq!(
            t.discovery_topic("light", &object_id),
            "homeassistant/light/ctl1/a0_vl_12/config"
        );
        assert!(t.unique_id(&object_id).ends_with(&object_id));
    }

    #[test]
    fn state_command_and_availability_topics_are_stable() {
        let t = topics();
        assert_eq!(t.lamp_state_topic(0, 12), "dali/ctl1/a0/vl/12/state");
        assert_eq!(t.lamp_command_topic(0, 12), "dali/ctl1/a0/vl/12/set");
        assert_eq!(t.group_state_topic(1, 7), "dali/ctl1/a1/group/7/state");
        assert_eq!(t.scene_select_command_topic(0), "dali/ctl1/a0/scene_select/set");
        assert_eq!(t.availability_topic(), "dali/ctl1/availability");
    }

    #[test]
    fn every_command_topic_we_publish_is_covered_by_a_subscription() {
        let t = topics();
        let subs = t.command_subscriptions();
        let matches = |filter: &str, topic: &str| {
            let f: Vec<&str> = filter.split('/').collect();
            let s: Vec<&str> = topic.split('/').collect();
            f.len() == s.len() && f.iter().zip(&s).all(|(a, b)| *a == "+" || a == b)
        };
        for topic in [
            t.lamp_command_topic(0, 12),
            t.group_command_topic(0, 7),
            t.scene_select_command_topic(0),
        ] {
            assert!(
                subs.iter().any(|f| matches(f, &topic)),
                "no subscription covers {topic}"
            );
        }
    }

    #[test]
    fn a_command_topic_round_trips_back_to_the_entity_it_addresses() {
        let t = topics();
        assert_eq!(
            t.parse_command_topic(&t.lamp_command_topic(0, 12)),
            Some(HaCommandTarget::VirtualLamp {
                adapter_id: 0,
                virtual_lamp_id: 12
            })
        );
        assert_eq!(
            t.parse_command_topic(&t.group_command_topic(1, 7)),
            Some(HaCommandTarget::Group {
                adapter_id: 1,
                group_id: 7
            })
        );
        assert_eq!(
            t.parse_command_topic(&t.scene_select_command_topic(0)),
            Some(HaCommandTarget::SceneSelect { adapter_id: 0 })
        );
    }

    #[test]
    fn topics_from_another_controller_or_of_another_shape_are_refused() {
        let t = topics();
        for topic in [
            "dali/other/a0/vl/12/set",
            "dali/ctl1/a0/vl/12/state",
            "dali/ctl1/a0/vl/set",
            "dali/ctl1/0/vl/12/set",
            "dali/ctl1/a0/vl/notanumber/set",
            "dali/ctl1/a0/unknown/12/set",
            "",
        ] {
            assert_eq!(t.parse_command_topic(topic), None, "accepted {topic}");
        }
    }

    #[test]
    fn a_controller_id_is_only_safe_inside_the_node_id_charset() {
        assert!(is_topic_safe("dali-a1b2c3"));
        assert!(is_topic_safe("ctl_1"));
        assert!(!is_topic_safe(""));
        assert!(!is_topic_safe("my controller"));
        assert!(!is_topic_safe("ctl.1"));
        assert!(!is_topic_safe("ctl/1"));
    }
}
