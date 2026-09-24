use crate::action::{Action, CctSpec, FlowAction, InputAction, LevelSpec, LightOp, SceneAction, StateAction};
use crate::condition::{Condition, VarOperand};
use crate::rule::RuleSet;
use crate::value::ValueExpr;
use std::collections::BTreeSet;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KindCoverage {
    pub triggers: BTreeSet<&'static str>,
    pub conditions: BTreeSet<&'static str>,
    pub actions: BTreeSet<&'static str>,
    pub values: BTreeSet<&'static str>,
}

impl KindCoverage {
    pub fn add(&mut self, set: &RuleSet) {
        for block in &set.blocks {
            self.add_actions(&block.actions);
        }
        for rule in &set.rules {
            for trigger in &rule.triggers {
                self.triggers.insert(trigger.kind().name());
            }
            for condition in &rule.conditions {
                self.add_condition(condition);
            }
            self.add_actions(&rule.actions);
        }
    }

    fn add_condition(&mut self, condition: &Condition) {
        self.conditions.insert(condition.kind().name());
        match condition {
            Condition::LampLevel { value, .. }
            | Condition::LampCct { value, .. }
            | Condition::InputLight { threshold: value, .. } => self.add_expr(value),
            Condition::VarCompare { value: VarOperand::Value(expr), .. } => self.add_expr(expr),
            _ => {}
        }
    }

    fn add_actions(&mut self, actions: &[Action]) {
        for action in actions {
            self.actions.insert(action.kind().name());
            match action {
                Action::Light(light) => self.add_light_values(&light.op),
                Action::Scene(scene) => self.add_scene_values(scene),
                Action::Input(input) => self.add_input_values(input),
                Action::Flow(flow) => self.add_flow(flow),
                Action::State(state) => self.add_state_values(state),
                Action::Hcl(_) => {}
            }
        }
    }

    fn add_light_values(&mut self, op: &LightOp) {
        match op {
            LightOp::On { level: Some(expr) } | LightOp::Toggle { level: Some(expr) } => {
                self.add_expr(expr);
            }
            LightOp::Level { level: LevelSpec::Absolute(expr) } => self.add_expr(expr),
            LightOp::Cct { cct: CctSpec::Absolute(expr) } => self.add_expr(expr),
            _ => {}
        }
    }

    fn add_scene_values(&mut self, scene: &SceneAction) {
        match scene {
            SceneAction::Recall { scene, .. } | SceneAction::Apply { scene } => {
                self.add_expr(scene);
            }
            SceneAction::Cycle { .. } => {}
        }
    }

    fn add_input_values(&mut self, input: &InputAction) {
        if let InputAction::PanelSelect { selected, .. } = input {
            self.add_expr(selected);
        }
    }

    fn add_state_values(&mut self, state: &StateAction) {
        if let StateAction::VarAdd { delta, .. } = state {
            self.add_expr(delta);
        }
    }

    fn add_flow(&mut self, flow: &FlowAction) {
        match flow {
            FlowAction::After { actions, .. } | FlowAction::Repeat { actions, .. } => {
                self.add_actions(actions);
            }
            FlowAction::Conditional { condition, then_actions, else_actions } => {
                self.add_condition(condition);
                self.add_actions(then_actions);
                self.add_actions(else_actions);
            }
            _ => {}
        }
    }

    fn add_expr(&mut self, expr: &ValueExpr) {
        if let Some(reading) = expr.reading() {
            self.values.insert(reading.kind().name());
        }
    }
}
