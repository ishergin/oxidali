use std::collections::HashMap;

use dali2rust_contracts::msg::{
    BusEventPayload, DaliInputEventObservedEvent, InputEventKind as WireKind,
    RuntimeStateChangedEvent,
};

use crate::runtime::engine::{EngineInput, InputEventKind};
use crate::runtime::world_port::RulesWorldPort;

fn button_kind(code: u16) -> Option<InputEventKind> {
    Some(match code {
        0x000 => InputEventKind::Release,
        0x001 => InputEventKind::Press,
        0x002 => InputEventKind::ShortPress,
        0x005 => InputEventKind::DoublePress,
        0x009 => InputEventKind::LongPressStart,
        0x00B => InputEventKind::LongPressRepeat,
        0x00C => InputEventKind::LongPressStop,
        0x00E => InputEventKind::ButtonFree,
        0x00F => InputEventKind::ButtonStuck,
        _ => return None,
    })
}

const OCCUPANCY_OCCUPIED_BIT: u16 = 1 << 1;
const OCCUPANCY_MOVEMENT_BIT: u16 = 1 << 0;

#[derive(Default)]
pub(crate) struct FunnelState {
    occupied: HashMap<(u8, u8, u8), bool>,
    lamps: HashMap<(u8, u16), (bool, u8)>,
    groups: HashMap<(u8, u16), bool>,
    devices: HashMap<(u8, u8), bool>,
    active: Option<bool>,
    hcl: HashMap<dali2rust_rules_model::LightTarget, bool>,
    manual_config: HashMap<(u8, u8, u8), bool>,
    watch_groups: bool,
}

impl FunnelState {
    pub(crate) fn map<'a>(
        &mut self,
        payload: &'a BusEventPayload,
        world: &dyn RulesWorldPort,
    ) -> Vec<EngineInput<'a>> {
        match payload {
            BusEventPayload::DaliInputEventObservedEvent(ev) => self.input_event(ev, world),
            BusEventPayload::RuntimeStateChangedEvent(ev) => {
                let mut inputs = self.lamp_event(ev);
                inputs.extend(self.group_edges(world));
                inputs
            }
            BusEventPayload::DaliSceneRecalledEvent(ev) => vec![EngineInput::SceneRecalled {
                adapter_id: ev.registry_adapter_id,
                scene_id: ev.scene_id,
            }],
            BusEventPayload::Dali103InstanceConfiguredEvent(ev) => self.manual_config_edge(ev),
            BusEventPayload::DaliInputDeviceLifecycleEvent(ev) => {
                vec![EngineInput::PowerCycled {
                    adapter_id: ev.registry_adapter_id,
                    short_address: ev.short_address,
                }]
            }
            BusEventPayload::DaliSettingsChangedEvent(_) => self.controller_edge(world),
            BusEventPayload::RedundancyTransitionEvent(ev) => {
                self.controller_edge_from(ev.now_active)
            }
            _ => Vec::new(),
        }
    }

    fn input_event<'a>(
        &mut self,
        ev: &'a DaliInputEventObservedEvent,
        world: &dyn RulesWorldPort,
    ) -> Vec<EngineInput<'a>> {
        let groups = match (ev.short_address, ev.instance_number) {
            (Some(short), Some(instance)) => {
                world.input_instance_groups(ev.registry_adapter_id, short, instance)
            }
            _ => [ev.instance_group, None, None],
        };
        let mut kinds: Vec<(InputEventKind, u16)> = Vec::new();
        match ev.typed {
            WireKind::Button => {
                let code = ev.typed_value;
                if let Some(kind) = button_kind(code) {
                    kinds.push((kind, code));
                }
            }
            WireKind::Occupancy => self.occupancy_kinds(ev, &mut kinds),
            WireKind::Illuminance => kinds.push((InputEventKind::LightCrossedAbove, ev.typed_value)),
            WireKind::Position => kinds.push((InputEventKind::PositionChanged, ev.typed_value)),
            WireKind::Generic => kinds.push((InputEventKind::Generic, ev.event_info)),
        }
        kinds
            .into_iter()
            .map(|(kind, value)| EngineInput::InputEvent {
                adapter_id: ev.registry_adapter_id,
                short_address: ev.short_address,
                instance_number: ev.instance_number,
                instance_type: ev.instance_type,
                instance_groups: groups,
                kind,
                value,
            })
            .collect()
    }

    fn occupancy_kinds(
        &mut self,
        ev: &DaliInputEventObservedEvent,
        kinds: &mut Vec<(InputEventKind, u16)>,
    ) {
        let occupied = ev.typed_value & OCCUPANCY_OCCUPIED_BIT != 0;
        let movement = ev.typed_value & OCCUPANCY_MOVEMENT_BIT != 0;
        kinds.push((
            if movement { InputEventKind::Movement } else { InputEventKind::NoMovement },
            ev.typed_value,
        ));
        if let (Some(short), Some(instance)) = (ev.short_address, ev.instance_number) {
            let key = (ev.registry_adapter_id, short, instance);
            let previous = self.occupied.insert(key, occupied);
            if previous != Some(occupied) && previous.is_some() {
                kinds.push((
                    if occupied {
                        InputEventKind::BecameOccupied
                    } else {
                        InputEventKind::BecameVacant
                    },
                    ev.typed_value,
                ));
            }
        }
    }

    fn lamp_event<'a>(&mut self, ev: &RuntimeStateChangedEvent) -> Vec<EngineInput<'a>> {
        let Some(lamp_id) = ev.virtual_lamp_id else {
            return Vec::new();
        };
        let is_on = matches!(
            ev.state_setpoint.power,
            dali2rust_contracts::msg::PowerState::On
        );
        let level = ev.state_setpoint.level;
        let key = (ev.adapter_id, u16::from(lamp_id));
        let (was_on, previous_level) = self.lamps.insert(key, (is_on, level)).unwrap_or((is_on, level));
        vec![EngineInput::LampChanged {
            adapter_id: ev.adapter_id,
            lamp_id: u16::from(lamp_id),
            is_on,
            level,
            was_on,
            previous_level,
        }]
    }

    fn manual_config_edge<'a>(
        &mut self,
        ev: &dali2rust_contracts::msg::Dali103InstanceConfiguredEvent,
    ) -> Vec<EngineInput<'a>> {
        let Some(active) = ev.manual_config_active else {
            return Vec::new();
        };
        let key = (ev.registry_adapter_id, ev.short_address, ev.instance_number);
        let previous = self.manual_config.insert(key, active);
        if previous.is_none() || previous == Some(active) {
            return Vec::new();
        }
        vec![EngineInput::ManualConfigChanged {
            adapter_id: ev.registry_adapter_id,
            short_address: ev.short_address,
        }]
    }

    fn controller_edge<'a>(&mut self, world: &dyn RulesWorldPort) -> Vec<EngineInput<'a>> {
        let active = world.controller_active();
        self.controller_edge_from(active)
    }

    fn controller_edge_from<'a>(&mut self, active: bool) -> Vec<EngineInput<'a>> {
        if self.active.replace(active) == Some(active) {
            return Vec::new();
        }
        let mut inputs = Vec::new();
        if active {
            inputs.push(EngineInput::ControllerStarts);
        }
        inputs.push(EngineInput::ControllerActive { active });
        inputs
    }

    pub(crate) fn seed_active(&mut self, active: bool) {
        self.active = Some(active);
    }

    pub(crate) fn set_watch_groups(&mut self, on: bool) {
        self.watch_groups = on;
    }

    pub(crate) fn group_edges<'a>(&mut self, world: &dyn RulesWorldPort) -> Vec<EngineInput<'a>> {
        if !self.watch_groups {
            return Vec::new();
        }
        let mut inputs = Vec::new();
        for group in world.groups() {
            let key = (group.adapter_id, group.id);
            let previous = self.groups.insert(key, group.any_on);
            if previous.is_some() && previous != Some(group.any_on) {
                inputs.push(EngineInput::GroupChanged {
                    adapter_id: group.adapter_id,
                    group_id: group.id,
                    any_on: group.any_on,
                    was_any_on: previous.unwrap_or(!group.any_on),
                });
            }
        }
        inputs
    }

    pub(crate) fn hcl_edges<'a>(&mut self, world: &dyn RulesWorldPort) -> Vec<EngineInput<'a>> {
        let mut inputs = Vec::new();
        for state in world.hcl() {
            let previous = self.hcl.insert(state.target, state.overridden);
            if previous.is_some() && previous != Some(state.overridden) {
                inputs.push(EngineInput::HclOverride {
                    started: state.overridden,
                    target: state.target,
                });
            }
        }
        inputs
    }

    pub(crate) fn tick_edges<'a>(
        &mut self,
        world: &dyn RulesWorldPort,
    ) -> Vec<EngineInput<'a>> {
        let mut inputs = self.group_edges(world);
        inputs.extend(self.hcl_edges(world));
        for device in world.devices() {
            let key = (device.adapter_id, device.short_address);
            let previous = self.devices.insert(key, device.online);
            if previous.is_some() && previous != Some(device.online) {
                inputs.push(EngineInput::DevicePresence {
                    adapter_id: device.adapter_id,
                    short_address: device.short_address,
                    online: device.online,
                });
            }
        }
        inputs
    }
}
