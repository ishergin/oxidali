use dali2rust_api::ha::{
    group_state_json, input_event_state_payload, light_state_json, scene_select_state,
};

use crate::runtime::discovery::{announce, input_entities, input_view, Entity, Gate};
use crate::runtime::ha_out::HaOut;
use crate::runtime::session::{PAYLOAD_OFFLINE, PAYLOAD_ONLINE};

pub(super) fn publish_runtime_state(out: &mut HaOut<'_>, body: &dali2rust_contracts::msg::RuntimeStateChangedEvent) {
    if let Some(lamp_id) = body.virtual_lamp_id {
        publish_lamp(out, body, lamp_id);
        return;
    }
    let Some(short_address) = body.short_address else {
        return;
    };
    for lamp_id in out.ports.read_port.ha_lamp_ids_for_short(body.adapter_id, short_address) {
        publish_lamp(out, body, lamp_id);
    }
}

fn publish_lamp(out: &mut HaOut<'_>, body: &dali2rust_contracts::msg::RuntimeStateChangedEvent, lamp_id: u8) {
    let adapter_id = body.adapter_id;
    let key = (adapter_id, Entity::Lamp(lamp_id));
    if !out.session.is_announced(key) {
        announce(out, adapter_id, Entity::Lamp(lamp_id), Gate::IfAnnounced);
    }
    if !out.session.is_announced(key) {
        return;
    }
    let absent = body.state_observation.reports_absence();
    publish_lamp_availability_if_changed(out, adapter_id, lamp_id, absent);
    if !body.state_setpoint.states_a_value() {
        return;
    }
    let topic = out.topics.lamp_state_topic(adapter_id, lamp_id);
    out.publish(&topic, &light_state_json(&body.state_setpoint), out.settings.retain_state);
}

pub(super) fn publish_lamp_availability(out: &mut HaOut<'_>, adapter_id: u8, lamp_id: u8, unreachable: bool) -> bool {
    let topic = out.topics.lamp_availability_topic(adapter_id, lamp_id);
    let payload = if unreachable { PAYLOAD_OFFLINE } else { PAYLOAD_ONLINE };
    let published = out.publish(&topic, payload, true);
    if published {
        out.session.lamp_availability.insert((adapter_id, lamp_id), unreachable);
    } else {
        out.session.lamp_availability.remove(&(adapter_id, lamp_id));
    }
    published
}

fn publish_lamp_availability_if_changed(out: &mut HaOut<'_>, adapter_id: u8, lamp_id: u8, unreachable: bool) {
    if out.session.lamp_availability.get(&(adapter_id, lamp_id)) == Some(&unreachable) {
        return;
    }
    publish_lamp_availability(out, adapter_id, lamp_id, unreachable);
}

pub(super) fn clear_lamp_retained(out: &mut HaOut<'_>, adapter_id: u8, lamp_id: u8) -> bool {
    out.session.lamp_availability.remove(&(adapter_id, lamp_id));
    let availability_topic = out.topics.lamp_availability_topic(adapter_id, lamp_id);
    let state_topic = out.topics.lamp_state_topic(adapter_id, lamp_id);
    let availability = out.publish(&availability_topic, "", true);
    let state = out.publish(&state_topic, "", true);
    availability && state
}

pub(super) fn publish_groups(out: &mut HaOut<'_>, adapter_id: u8) {
    let recheck = out.session.groups_dirty.remove(&adapter_id);
    for group in out.ports.read_port.ha_group_views(adapter_id) {
        let entity = Entity::Group(group.group_id);
        if recheck || !out.session.is_announced((adapter_id, entity)) {
            announce(out, adapter_id, entity, Gate::IfChanged);
        }
        let topic = out.topics.group_state_topic(adapter_id, group.group_id);
        out.publish(&topic, &group_state_json(&group.state), out.settings.retain_state);
    }
}

pub(super) fn publish_scene_select(out: &mut HaOut<'_>, adapter_id: u8) -> bool {
    let scenes = out.ports.read_port.ha_scene_views(adapter_id);
    if scenes.is_empty() {
        return false;
    }
    if !out.session.is_announced((adapter_id, Entity::SceneSelect)) {
        announce(out, adapter_id, Entity::SceneSelect, Gate::IfChanged);
    }
    let active = out.ports.read_port.ha_active_scene(adapter_id);
    let topic = out.topics.scene_select_state_topic(adapter_id);
    out.publish(&topic, &scene_select_state(active, &scenes), out.settings.retain_state);
    true
}

#[inline(never)]
pub(super) fn republish_scene_select(out: &mut HaOut<'_>, adapter_id: u8) {
    out.session.forget((adapter_id, Entity::SceneSelect));
    if !publish_scene_select(out, adapter_id) {
        announce(out, adapter_id, Entity::SceneSelect, Gate::IfChanged);
    }
}

#[inline(never)]
pub(super) fn publish_input_event(out: &mut HaOut<'_>, body: &dali2rust_contracts::msg::DaliInputEventObservedEvent) {
    let Some((short_address, instance_number, payload)) = exposed_input_event(out, body) else {
        return;
    };
    let adapter_id = body.registry_adapter_id;
    let key = (adapter_id, Entity::Input(short_address, instance_number));
    if !out.session.is_announced(key) {
        republish_input_configs(out, adapter_id, short_address);
    }
    if !out.session.is_announced(key) {
        return;
    }
    let retain = out.settings.retain_state
        && body.typed != dali2rust_contracts::msg::InputEventKind::Button;
    let topic = out.topics.input_state_topic(adapter_id, short_address, instance_number);
    out.publish(&topic, &payload, retain);
}

#[inline(never)]
fn exposed_input_event(
    out: &HaOut<'_>,
    body: &dali2rust_contracts::msg::DaliInputEventObservedEvent,
) -> Option<(u8, u8, String)> {
    if !out.settings.expose_input_devices {
        return None;
    }
    let (Some(short_address), Some(instance_number)) = (body.short_address, body.instance_number)
    else {
        return None;
    };
    let view = out.ports.read_port.ha_input_view(body.registry_adapter_id, short_address)?;
    if !view.ha_expose {
        return None;
    }
    let payload = input_event_state_payload(body.typed, body.typed_value)?;
    Some((short_address, instance_number, payload))
}

#[inline(never)]
pub(super) fn republish_input_configs(out: &mut HaOut<'_>, adapter_id: u8, short_address: u8) {
    let Some(view) = input_view(out.ports.read_port.as_ref(), adapter_id, short_address) else {
        return;
    };
    for entity in input_entities(&view) {
        announce(out, adapter_id, entity, Gate::IfChanged);
    }
}
