use std::sync::atomic::Ordering;

use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, PublishResult};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::{
    ColorMode, ColorValue, DaliRecallSceneCommand, DaliSetTargetStateCommand, DaliStopFadeCommand,
    DaliTargetScope, FixedText32, HclOverrideClearCommand, HclOverrideHoldCommand,
    HclOverrideTarget, HclScheduleEnableCommand, LightSetpoint, MqttPublishCommand, Origin,
    PowerState, SceneApplyExecuteCommand,
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

impl ExecutionReport {
    fn note(&mut self, published: bool) {
        if published {
            self.executed = self.executed.saturating_add(1);
        } else {
            self.failed = self.failed.saturating_add(1);
        }
    }
}

struct MergedLight {
    at: usize,
    target: LightTarget,
    setpoint: LightSetpoint,
    hold_hcl: bool,
    bound: bool,
}

impl MergedLight {
    fn absorb(&mut self, later: &MergedLight) {
        self.setpoint.merge_from(&later.setpoint);
    }
}

#[derive(Default)]
struct LightSeries {
    lights: Vec<MergedLight>,
    open: Vec<(LightKey, usize)>,
}

impl LightSeries {
    fn close(&mut self) {
        self.open.clear();
    }

    fn absorb(&mut self, light: MergedLight) {
        let Some(key) = light_key(&light.target) else {
            self.lights.push(light);
            return;
        };
        match self.open.iter_mut().find(|(open, _)| *open == key) {
            Some((_, slot)) if self.lights[*slot].hold_hcl == light.hold_hcl => {
                self.lights[*slot].absorb(&light);
            }
            Some((_, slot)) => {
                *slot = self.lights.len();
                self.lights.push(light);
            }
            None => {
                self.open.push((key, self.lights.len()));
                self.lights.push(light);
            }
        }
    }
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
        let mut report = ExecutionReport::default();
        let mut lights = self.merge_lights(effects, snapshot);
        for (index, effect) in effects.iter().enumerate() {
            let published = match effect {
                Effect::Light { target, verb: LightVerb::StopFade, .. } => {
                    self.stop_fade(target, snapshot, corr)
                }
                Effect::Light { .. } => match lights.iter().position(|light| light.at == index) {
                    Some(slot) => self.publish_light(lights.remove(slot), corr),
                    None => continue,
                },
                other => self.one(other, snapshot, corr),
            };
            report.note(published);
        }
        report
    }

    fn merge_lights(&self, effects: &[Effect], snapshot: &WorldSnapshot) -> Vec<MergedLight> {
        let mut series = LightSeries::default();
        for (index, effect) in effects.iter().enumerate() {
            let Effect::Light { target, verb, hold_hcl } = effect else {
                series.close();
                continue;
            };
            if matches!(verb, LightVerb::StopFade) {
                series.close();
                continue;
            }
            if let Some(light) = self.plan_light(index, target, verb, *hold_hcl, snapshot) {
                series.absorb(light);
            }
        }
        series.lights
    }

    fn plan_light(
        &self,
        at: usize,
        target: &LightTarget,
        verb: &LightVerb,
        hold_hcl: bool,
        snapshot: &WorldSnapshot,
    ) -> Option<MergedLight> {
        let bound = lamp_bound(target, snapshot);
        let setpoint = match setpoint_of(verb, target, snapshot) {
            Some(setpoint) => setpoint,
            None if bound => {
                self.counters.effects_skipped_dark.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            None => LightSetpoint::default(),
        };
        Some(MergedLight { at, target: *target, setpoint, hold_hcl, bound })
    }

    fn one(&self, effect: &Effect, snapshot: &WorldSnapshot, corr: u64) -> bool {
        match effect {
            Effect::Light { .. } => self.misrouted(),
            Effect::SceneRecall { .. } | Effect::SceneApply { .. } => {
                self.scene_effect(effect, snapshot, corr)
            }
            Effect::HclResume { .. } | Effect::HclHold { .. } | Effect::HclSchedule { .. } => {
                self.hcl_effect(effect, snapshot, corr)
            }
            Effect::InputFeedback { .. }
            | Effect::PanelSelect { .. }
            | Effect::CancelHold { .. }
            | Effect::CatchMovement { .. } => self.input_effect(effect, corr),
            Effect::MqttPublish { .. } | Effect::Log { .. } | Effect::StatCount { .. } => {
                self.state_effect(effect, corr)
            }
        }
    }

    fn misrouted(&self) -> bool {
        debug_assert!(false, "an effect reached the executor of another family");
        false
    }

    fn scene_effect(&self, effect: &Effect, snapshot: &WorldSnapshot, corr: u64) -> bool {
        match effect {
            Effect::SceneRecall { scene, target, hold_hcl } => {
                self.scene_recall(*scene, target, *hold_hcl, snapshot, corr)
            }
            Effect::SceneApply { scene, .. } => self.scene_apply(*scene, corr),
            _ => self.misrouted(),
        }
    }

    fn hcl_effect(&self, effect: &Effect, snapshot: &WorldSnapshot, corr: u64) -> bool {
        match effect {
            Effect::HclResume { target } => self.hcl_resume(target, corr),
            Effect::HclHold { target } => self.hcl_hold(target, snapshot, corr),
            Effect::HclSchedule { schedule, enabled } => self.hcl_switch(schedule, *enabled, corr),
            _ => self.misrouted(),
        }
    }

    fn input_effect(&self, effect: &Effect, corr: u64) -> bool {
        match effect {
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
            _ => self.misrouted(),
        }
    }

    fn state_effect(&self, effect: &Effect, corr: u64) -> bool {
        match effect {
            Effect::MqttPublish { topic, payload, retain } => {
                self.mqtt(topic, payload, *retain, corr)
            }
            Effect::Log { text: _ } => self.counted(&self.counters.log_lines),
            Effect::StatCount { .. } => self.counted(&self.counters.stat_counts),
            _ => self.misrouted(),
        }
    }

    fn counted(&self, cell: &std::sync::atomic::AtomicU32) -> bool {
        cell.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn publish_light(&self, light: MergedLight, corr: u64) -> bool {
        let Some((scope, virtual_lamp_id, group_id, adapter)) = scope_of(&light.target) else {
            return false;
        };
        if !light.bound {
            return self.unbound_lamp();
        }
        self.publish(
            corr,
            DaliSetTargetStateCommand {
                scope,
                virtual_lamp_id,
                short_address: 0,
                group_id,
                setpoint: light.setpoint,
                registry_adapter_id: adapter,
                hold_hcl: light.hold_hcl,
            },
        )
    }

    // IEC 62386-102 §9.5.9
    fn stop_fade(&self, target: &LightTarget, snapshot: &WorldSnapshot, corr: u64) -> bool {
        let Some((scope, virtual_lamp_id, group_id, adapter)) = scope_of(target) else {
            return false;
        };
        if !lamp_bound(target, snapshot) {
            return self.unbound_lamp();
        }
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

    fn scene_recall(
        &self,
        scene: u8,
        target: &Option<LightTarget>,
        hold_hcl: bool,
        snapshot: &WorldSnapshot,
        corr: u64,
    ) -> bool {
        let Some(command) = recall_command(scene, target) else {
            return false;
        };
        if target.is_some_and(|target| !lamp_bound(&target, snapshot)) {
            return self.unbound_lamp();
        }
        self.publish(corr, DaliRecallSceneCommand { hold_hcl, ..command })
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

    fn hcl_hold(&self, target: &LightTarget, snapshot: &WorldSnapshot, corr: u64) -> bool {
        let Some((registry_adapter_id, held)) = override_target_of(target) else {
            return false;
        };
        if !lamp_bound(target, snapshot) {
            return self.unbound_lamp();
        }
        self.publish(corr, HclOverrideHoldCommand { registry_adapter_id, target: held })
    }

    fn hcl_switch(&self, schedule: &str, enabled: bool, corr: u64) -> bool {
        let Some(schedule_id) = schedule_id_of(schedule) else {
            return false;
        };
        self.publish(corr, HclScheduleEnableCommand { schedule_id, enabled })
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

    fn unbound_lamp(&self) -> bool {
        self.counters.effects_unbound.fetch_add(1, Ordering::Relaxed);
        false
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

fn light_key(target: &LightTarget) -> Option<LightKey> {
    let (scope, virtual_lamp_id, group_id, adapter) = scope_of(target)?;
    Some(LightKey {
        scope: scope as u8,
        id: virtual_lamp_id.max(group_id),
        adapter,
    })
}

fn setpoint_of(
    verb: &LightVerb,
    target: &LightTarget,
    snapshot: &WorldSnapshot,
) -> Option<LightSetpoint> {
    let current = lamp_of(target, snapshot);
    let powered = |power| LightSetpoint { power, ..LightSetpoint::default() };
    Some(match verb {
        LightVerb::On { level } => LightSetpoint { level: *level, ..powered(PowerState::On) },
        LightVerb::Off => powered(PowerState::Off),
        LightVerb::Level { level } => LightSetpoint::from_level(*level, None),
        LightVerb::LevelRelative { delta } | LightVerb::Dim { delta } => {
            let lamp = current.filter(|lamp| lamp.is_on)?;
            LightSetpoint::from_level(clamp_level(i32::from(lamp.level?) + i32::from(*delta)), None)
        }
        LightVerb::Cct { .. }
        | LightVerb::CctRelative { .. }
        | LightVerb::Xy { .. }
        | LightVerb::Rgb { .. } => LightSetpoint {
            color: Some(colour_of(verb, current)?),
            ..LightSetpoint::default()
        },
        LightVerb::LastActive => powered(PowerState::On),
        LightVerb::StopFade => return None,
    })
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

fn lamp_bound(target: &LightTarget, snapshot: &WorldSnapshot) -> bool {
    match target {
        LightTarget::Lamp(_) => lamp_of(target, snapshot).is_some_and(|lamp| lamp.bound),
        LightTarget::Group(_) | LightTarget::Broadcast { .. } => true,
    }
}

fn recall_command(scene: u8, target: &Option<LightTarget>) -> Option<DaliRecallSceneCommand> {
    Some(match target {
        Some(LightTarget::Group(group)) => {
            DaliRecallSceneCommand::for_group(group.adapter_id, u8::try_from(group.id).ok()?, scene)
        }
        Some(LightTarget::Lamp(lamp)) => DaliRecallSceneCommand::for_virtual_lamp(
            lamp.adapter_id,
            u8::try_from(lamp.id).ok()?,
            scene,
        ),
        Some(LightTarget::Broadcast { adapter_id }) => {
            DaliRecallSceneCommand::broadcast(*adapter_id, scene)
        }
        None => DaliRecallSceneCommand::broadcast(0, scene),
    })
}

fn override_target_of(target: &LightTarget) -> Option<(u8, HclOverrideTarget)> {
    Some(match target {
        LightTarget::Lamp(lamp) => (
            lamp.adapter_id,
            HclOverrideTarget::VirtualLamp { virtual_lamp_id: u8::try_from(lamp.id).ok()? },
        ),
        LightTarget::Group(group) => (
            group.adapter_id,
            HclOverrideTarget::Group { group_id: u8::try_from(group.id).ok()? },
        ),
        LightTarget::Broadcast { adapter_id } => (*adapter_id, HclOverrideTarget::Broadcast),
    })
}

fn schedule_id_of(name: &str) -> Option<FixedText32> {
    let mut id = FixedText32::new();
    id.push_str(name).ok()?;
    Some(id)
}

fn scope_of(target: &LightTarget) -> Option<(DaliTargetScope, u8, u8, u8)> {
    Some(match target {
        LightTarget::Lamp(lamp) => (
            DaliTargetScope::VirtualLamp,
            u8::try_from(lamp.id).ok()?,
            0,
            lamp.adapter_id,
        ),
        LightTarget::Group(group) => {
            (DaliTargetScope::Group, 0, u8::try_from(group.id).ok()?, group.adapter_id)
        }
        LightTarget::Broadcast { adapter_id } => (DaliTargetScope::Broadcast, 0, 0, *adapter_id),
    })
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn sp(power: PowerState, level: Option<u8>, color: Option<ColorValue>) -> LightSetpoint {
        LightSetpoint { power, level, color }
    }

    #[test]
    fn a_level_and_a_colour_on_one_lamp_become_one_setpoint() {
        let mut acc = sp(PowerState::On, Some(200), None);
        let colour_only = sp(PowerState::Unknown, None, Some(ColorValue::default()));
        acc.merge_from(&colour_only);
        assert_eq!(acc.power, PowerState::On, "colour must not clear power");
        assert_eq!(acc.level, Some(200), "colour must not clear the level");
        assert!(acc.color.is_some(), "the colour is carried too");
    }

    #[test]
    fn a_later_power_verb_wins_and_carries_its_own_level() {
        let mut acc = sp(PowerState::On, Some(200), Some(ColorValue::default()));
        acc.merge_from(&sp(PowerState::Off, Some(0), None));
        assert_eq!(acc.power, PowerState::Off);
        assert_eq!(acc.level, Some(0));
        assert!(acc.color.is_some(), "a power verb says nothing about colour");
    }

    #[test]
    fn a_later_level_does_not_erase_a_stated_colour() {
        let stated = ColorValue {
            mode: ColorMode::Cct,
            ..ColorValue::default()
        };
        let mut acc = sp(PowerState::On, None, Some(stated));
        acc.merge_from(&sp(PowerState::On, Some(200), Some(ColorValue::default())));
        assert_eq!(acc.level, Some(200), "the later level wins");
        assert_eq!(
            acc.color.map(|c| c.mode),
            Some(ColorMode::Cct),
            "a setpoint that states no colour must leave the stated one alone"
        );
    }

    fn light(at: usize, target: LightTarget, setpoint: LightSetpoint) -> MergedLight {
        MergedLight { at, target, setpoint, hold_hcl: true, bound: true }
    }

    #[test]
    fn a_target_whose_id_does_not_fit_its_field_opens_no_merge_slot() {
        let wide = LightTarget::Lamp(dali2rust_rules_model::LampRef { id: 300, adapter_id: 0 });
        let mut series = LightSeries::default();
        series.absorb(light(0, wide, sp(PowerState::Off, None, None)));
        series.absorb(light(1, wide, sp(PowerState::On, Some(10), None)));
        assert_eq!(series.lights.len(), 2, "each effect stays apart and fails on its own");
        assert!(series.open.is_empty(), "no merge slot for a key that cannot be built");
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
