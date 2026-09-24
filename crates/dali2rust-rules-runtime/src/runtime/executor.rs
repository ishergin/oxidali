use std::sync::atomic::Ordering;

use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    ColorMode, ColorValue, DaliRecallSceneCommand, DaliSetTargetStateCommand, DaliStopFadeCommand,
    DaliTargetScope,
    HclOverrideClearCommand, LightSetpoint, MqttPublishCommand, Origin, PowerState,
    SceneApplyExecuteCommand,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_rules_model::LightTarget;

use crate::runtime::engine::{Effect, LightVerb, WorldSnapshot};
use crate::runtime::worker::RulesWorkerCounters;
use crate::runtime::world_port::RulesWorldPort;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ExecutionReport {
    pub executed: u8,
    pub failed: u8,
}

pub(crate) struct EffectExecutor<'a> {
    pub publisher: &'a BusPublisher,
    pub bus_id: BusId,
    pub world: &'a dyn RulesWorldPort,
    pub counters: &'a RulesWorkerCounters,
}

impl EffectExecutor<'_> {
    pub(crate) fn execute(
        &self,
        effects: &[Effect],
        snapshot: &WorldSnapshot,
        corr: u64,
    ) -> ExecutionReport {
        let mut executed: u8 = 0;
        let mut failed: u8 = 0;
        let mut lights = self.merge_lights(effects, snapshot);
        for (index, effect) in effects.iter().enumerate() {
            let published = match effect {
                Effect::Light { target, verb: LightVerb::StopFade, .. } => {
                    self.stop_fade(target, corr)
                }
                Effect::Light { .. } => match lights.iter().position(|(at, _, _)| *at == index) {
                    Some(slot) => {
                        let (_, target, setpoint) = lights.remove(slot);
                        self.publish_light(&target, setpoint, corr)
                    }
                    None => continue,
                },
                other => self.one(other, snapshot, corr),
            };
            if published {
                executed = executed.saturating_add(1);
            } else {
                failed = failed.saturating_add(1);
            }
        }
        ExecutionReport { executed, failed }
    }

    fn merge_lights(
        &self,
        effects: &[Effect],
        snapshot: &WorldSnapshot,
    ) -> Vec<(usize, LightTarget, LightSetpoint)> {
        let mut lights: Vec<(usize, LightTarget, LightSetpoint)> = Vec::new();
        let mut open: Vec<(LightKey, usize)> = Vec::new();
        for (index, effect) in effects.iter().enumerate() {
            let Effect::Light { target, verb, .. } = effect else {
                open.clear();
                continue;
            };
            if matches!(verb, LightVerb::StopFade) {
                open.clear();
                continue;
            }
            let Some(sp) = setpoint_of(verb, target, snapshot) else {
                self.counters.effects_skipped_dark.fetch_add(1, Ordering::Relaxed);
                continue;
            };
            let key = light_key(target);
            let slot = open.iter().find(|(k, _)| *k == key).map(|(_, slot)| *slot);
            match slot {
                Some(slot) => lights[slot].2.merge_from(&sp),
                None => {
                    open.push((key, lights.len()));
                    lights.push((index, *target, sp));
                }
            }
        }
        lights
    }

    fn one(&self, effect: &Effect, _snapshot: &WorldSnapshot, corr: u64) -> bool {
        match effect {
            Effect::Light { .. } => {
                debug_assert!(false, "light effects are merged and published by execute()");
                false
            }
            Effect::SceneRecall { scene, target, .. } => self.scene_recall(*scene, target, corr),
            Effect::SceneApply { scene, .. } => self.scene_apply(*scene, corr),
            Effect::HclResume { target } => self.hcl_resume(target, corr),
            Effect::HclHold { .. } => self.unmapped(&self.counters.hcl_hold_unmapped),
            Effect::HclSchedule { .. } => self.unmapped(&self.counters.hcl_schedule_unmapped),
            Effect::InputFeedback { input, on } => self.feedback_drive(
                Some(input.device_short_address),
                Some(input.instance_number),
                None,
                if *on { 0 } else { 1 },
                0,
                input.adapter_id,
                corr,
            ),
            Effect::PanelSelect { adapter_id, group, selected } => {
                self.feedback_drive(None, None, Some(*group), 2, *selected, *adapter_id, corr)
            }
            Effect::CancelHold { .. } | Effect::CatchMovement { .. } => {
                self.unmapped(&self.counters.input_action_unmapped)
            }
            Effect::MqttPublish { topic, payload, retain } => self.mqtt(topic, payload, *retain, corr),
            Effect::Log { text: _ } => {
                self.counters.log_lines.fetch_add(1, Ordering::Relaxed);
                true
            }
            Effect::StatCount { .. } => {
                self.counters.stat_counts.fetch_add(1, Ordering::Relaxed);
                true
            }
        }
    }

    fn publish_light(&self, target: &LightTarget, setpoint: LightSetpoint, corr: u64) -> bool {
        let (scope, virtual_lamp_id, group_id, adapter) = scope_of(target);
        self.publish(
            corr,
            DaliSetTargetStateCommand {
                scope,
                virtual_lamp_id,
                short_address: 0,
                group_id,
                setpoint,
                registry_adapter_id: adapter,
            },
        )
    }

    // IEC 62386-102 §9.5.9
    fn stop_fade(&self, target: &LightTarget, corr: u64) -> bool {
        let (scope, virtual_lamp_id, group_id, adapter) = scope_of(target);
        self.publish(
            corr,
            DaliStopFadeCommand {
                registry_adapter_id: adapter,
                scope,
                virtual_lamp_id,
                short_address: 0,
                group_id,
            },
        )
    }

    fn scene_recall(&self, scene: u8, target: &Option<LightTarget>, corr: u64) -> bool {
        let (scope, group_id, adapter) = match target {
            Some(LightTarget::Group(group)) => {
                (DaliTargetScope::Group, group.id as u8, group.adapter_id)
            }
            Some(LightTarget::Broadcast { adapter_id }) => {
                (DaliTargetScope::Broadcast, 0, *adapter_id)
            }
            Some(LightTarget::Lamp(_)) => {
                return self.unmapped(&self.counters.input_action_unmapped);
            }
            None => (DaliTargetScope::Broadcast, 0, 0),
        };
        self.publish(
            corr,
            DaliRecallSceneCommand {
                registry_adapter_id: adapter,
                scope,
                short_address: 0,
                group_id,
                scene_id: scene,
            },
        )
    }

    fn scene_apply(&self, scene: u8, corr: u64) -> bool {
        self.publish(
            corr,
            SceneApplyExecuteCommand {
                registry_adapter_id: 0,
                scene_id: scene,
                operation_key: dali2rust_contracts::msg::fixed_text_32(&format!(
                    "rule-scn-{scene}-{corr}"
                )),
            },
        )
    }

    fn hcl_resume(&self, target: &LightTarget, corr: u64) -> bool {
        let schedules = self.world.hcl_schedules_for(target);
        if schedules.is_empty() {
            return false;
        }
        let mut any = false;
        for schedule in schedules {
            any |= self.publish(
                corr,
                HclOverrideClearCommand {
                    schedule_id: dali2rust_contracts::msg::fixed_text_32(&schedule),
                },
            );
        }
        any
    }

    #[allow(clippy::too_many_arguments, reason = "one private call shape for both drive forms")]
    fn feedback_drive(
        &self,
        feature_number: Option<u8>,
        instance: Option<u8>,
        feature_group: Option<u8>,
        action: u8,
        selected: u8,
        adapter: u8,
        corr: u64,
    ) -> bool {
        let _ = feature_number;
        self.publish(
            corr,
            dali2rust_contracts::msg::Dali103FeedbackDriveCommand {
                registry_adapter_id: adapter,
                action,
                short_address: feature_number,
                feature_number: instance,
                feature_group,
                selected_group: selected,
                opcode_map: 0,
            },
        )
    }

    fn mqtt(&self, topic: &str, payload: &str, retain: bool, corr: u64) -> bool {
        self.publish(
            corr,
            MqttPublishCommand {
                topic: dali2rust_contracts::msg::fixed_text_48(topic),
                payload: dali2rust_contracts::msg::fixed_text_48(payload),
                retain,
            },
        )
    }

    fn unmapped(&self, cell: &std::sync::atomic::AtomicU32) -> bool {
        cell.fetch_add(1, Ordering::Relaxed);
        false
    }

    fn publish<P>(&self, corr: u64, payload: P) -> bool
    where
        dali2rust_contracts::msg::BusCommandPayload: From<P>,
    {
        let env = command_envelope(
            SOURCE_ID_UNSPECIFIED,
            corr,
            self.bus_id.0,
            Some(Origin::Rules),
            payload,
        );
        let queued = self.publisher.try_publish(BusChannel::Commands, BusFrame::command(env))
            == PublishResult::Queued;
        if queued {
            self.counters.effects_published.fetch_add(1, Ordering::Relaxed);
        } else {
            self.counters.effects_ingress_rejected.fetch_add(1, Ordering::Relaxed);
        }
        queued
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct LightKey {
    scope: u8,
    id: u8,
    adapter: u8,
}

fn light_key(target: &LightTarget) -> LightKey {
    let (scope, virtual_lamp_id, group_id, adapter) = scope_of(target);
    LightKey {
        scope: scope as u8,
        id: virtual_lamp_id.max(group_id),
        adapter,
    }
}

fn setpoint_of(
    verb: &LightVerb,
    target: &LightTarget,
    snapshot: &WorldSnapshot,
) -> Option<LightSetpoint> {
    let current = lamp_of(target, snapshot);
    let mut sp = LightSetpoint { power: PowerState::Unknown, level: 0, color: None };
    match verb {
        LightVerb::On { level } => {
            sp.power = PowerState::On;
            sp.level = level.unwrap_or(0);
        }
        LightVerb::Off => sp.power = PowerState::Off,
        LightVerb::Level { level } => {
            sp.power = PowerState::On;
            sp.level = *level;
        }
        LightVerb::LevelRelative { delta } | LightVerb::Dim { delta } => {
            let lamp = current?;
            if !lamp.is_on {
                return None;
            }
            sp.power = PowerState::On;
            sp.level = clamp_level(i32::from(lamp.level) + i32::from(*delta));
        }
        LightVerb::Cct { .. }
        | LightVerb::CctRelative { .. }
        | LightVerb::Xy { .. }
        | LightVerb::Rgb { .. } => sp.color = Some(colour_of(verb, current)?),
        LightVerb::LastActive => sp.power = PowerState::On,
        LightVerb::StopFade => return None,
    }
    Some(sp)
}

fn colour_of(
    verb: &LightVerb,
    current: Option<&crate::runtime::engine::LampState>,
) -> Option<ColorValue> {
    Some(match verb {
        LightVerb::Cct { kelvin } => cct(*kelvin),
        LightVerb::CctRelative { delta_k } => {
            let base = current?.cct_kelvin?;
            cct(clamp_kelvin(i64::from(base) + i64::from(*delta_k)))
        }
        LightVerb::Xy { x_1e4, y_1e4 } => ColorValue {
            mode: ColorMode::Xy,
            x: *x_1e4,
            y: *y_1e4,
            ..cct(0)
        },
        LightVerb::Rgb { r, g, b } => {
            ColorValue { mode: ColorMode::Rgb, r: *r, g: *g, b: *b, ..cct(0) }
        }
        _ => return None,
    })
}

fn cct(kelvin: u16) -> ColorValue {
    ColorValue {
        mode: if kelvin == 0 { ColorMode::None } else { ColorMode::Cct },
        color_temperature_kelvin: kelvin,
        x: 0,
        y: 0,
        r: 0,
        g: 0,
        b: 0,
        w: 0,
        a: 0,
        f: 0,
    }
}

fn clamp_level(level: i32) -> u8 {
    level.clamp(0, 254) as u8
}

fn clamp_kelvin(kelvin: i64) -> u16 {
    kelvin.clamp(1000, 20_000) as u16
}

fn lamp_of<'a>(
    target: &LightTarget,
    snapshot: &'a WorldSnapshot,
) -> Option<&'a crate::runtime::engine::LampState> {
    match target {
        LightTarget::Lamp(lamp) => snapshot
            .lamps
            .iter()
            .find(|l| l.adapter_id == lamp.adapter_id && l.id == lamp.id),
        _ => None,
    }
}

fn scope_of(target: &LightTarget) -> (DaliTargetScope, u8, u8, u8) {
    match target {
        LightTarget::Lamp(lamp) => (
            DaliTargetScope::VirtualLamp,
            lamp.id as u8,
            0,
            lamp.adapter_id,
        ),
        LightTarget::Group(group) => (DaliTargetScope::Group, 0, group.id as u8, group.adapter_id),
        LightTarget::Broadcast { adapter_id } => (DaliTargetScope::Broadcast, 0, 0, *adapter_id),
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn sp(power: PowerState, level: u8, color: Option<ColorValue>) -> LightSetpoint {
        LightSetpoint { power, level, color }
    }

    #[test]
    fn a_level_and_a_colour_on_one_lamp_become_one_setpoint() {
        let mut acc = sp(PowerState::On, 200, None);
        let colour_only = sp(PowerState::Unknown, 0, Some(ColorValue::default()));
        acc.merge_from(&colour_only);
        assert_eq!(acc.power, PowerState::On, "colour must not clear power");
        assert_eq!(acc.level, 200, "colour must not clear the level");
        assert!(acc.color.is_some(), "the colour is carried too");
    }

    #[test]
    fn a_later_power_verb_wins_and_carries_its_own_level() {
        let mut acc = sp(PowerState::On, 200, Some(ColorValue::default()));
        acc.merge_from(&sp(PowerState::Off, 0, None));
        assert_eq!(acc.power, PowerState::Off);
        assert_eq!(acc.level, 0);
        assert!(acc.color.is_some(), "a power verb says nothing about colour");
    }

    #[test]
    fn a_later_level_does_not_erase_a_stated_colour() {
        let stated = ColorValue {
            mode: ColorMode::Cct,
            ..ColorValue::default()
        };
        let mut acc = sp(PowerState::On, 0, Some(stated));
        acc.merge_from(&sp(PowerState::On, 200, Some(ColorValue::default())));
        assert_eq!(acc.level, 200, "the later level wins");
        assert_eq!(
            acc.color.map(|c| c.mode),
            Some(ColorMode::Cct),
            "a setpoint that states no colour must leave the stated one alone"
        );
    }

    #[test]
    fn different_targets_do_not_merge() {
        let a = light_key(&LightTarget::Lamp(dali2rust_rules_model::LampRef {
            id: 1,
            adapter_id: 0,
        }));
        let b = light_key(&LightTarget::Lamp(dali2rust_rules_model::LampRef {
            id: 2,
            adapter_id: 0,
        }));
        assert!(a != b);
    }
}
