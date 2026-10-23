use std::hash::{Hash, Hasher};

use dali2rust_api::ha::{
    group_discovery_json, input_instance_discovery_json, lamp_discovery_json,
    scene_select_discovery_json, HaTopics, DISCOVERY_RETRACTION,
};
use dali2rust_bus::worker_counters::bump;
use dali2rust_contracts::msg::{ErrorCode, OperationWorkerSignalEvent};
use dali2rust_domain::registry::{HaPublishReadPort, HomeAssistantSettingsView};

use crate::runtime::bus_out::publish_event;
use crate::runtime::ha_out::HaOut;
use crate::runtime::state::{clear_lamp_retained, publish_lamp_availability};
use crate::runtime::worker::{MqttWorkerPorts, WORKER_STACK};

const DISCOVERY_BATCH: usize = 4;

const LIGHT: &str = "light";

const SELECT: &str = "select";

pub(super) struct DiscoveryJob {
    pending: Vec<(u8, Entity)>,
    published: u16,
    failed: u16,
    pub(super) workflow: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Entity {
    Lamp(u8),
    Group(u8),
    Input(u8, u8),
    SceneSelect,
}

pub(super) type EntityKey = (u8, Entity);

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Gate {
    Force,
    IfChanged,
    IfAnnounced,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Announced {
    Skipped,
    Published,
    Refused,
}

impl From<bool> for Announced {
    fn from(delivered: bool) -> Self {
        if delivered {
            Self::Published
        } else {
            Self::Refused
        }
    }
}

#[inline(never)]
pub(super) fn plan_discovery(ports: &MqttWorkerPorts, adapter_count: u8, workflow: Option<u64>) -> DiscoveryJob {
    let mut pending = Vec::new();
    for adapter_id in 0..adapter_count {
        plan_adapter(&mut pending, ports.read_port.as_ref(), adapter_id);
    }
    pending.reverse();
    DiscoveryJob {
        pending,
        published: 0,
        failed: 0,
        workflow,
    }
}

fn plan_adapter(pending: &mut Vec<(u8, Entity)>, port: &dyn HaPublishReadPort, adapter_id: u8) {
    let mut plan = |entity| pending.push((adapter_id, entity));
    for view in port.ha_input_views(adapter_id) {
        for instance in &view.instances {
            if instance.instance_type.and_then(input_component).is_some() {
                plan(Entity::Input(view.short_address, instance.instance_number));
            }
        }
    }
    port.ha_lamp_views(adapter_id).iter().for_each(|lamp| plan(Entity::Lamp(lamp.virtual_lamp_id)));
    port.ha_group_views(adapter_id).iter().for_each(|group| plan(Entity::Group(group.group_id)));
    plan(Entity::SceneSelect);
    port.ha_retracted_lamp_ids(adapter_id).into_iter().for_each(|id| plan(Entity::Lamp(id)));
    port.ha_retracted_group_ids(adapter_id).into_iter().for_each(|id| plan(Entity::Group(id)));
}

pub(super) fn advance_job(out: &mut HaOut<'_>, job: &mut Option<DiscoveryJob>) {
    let Some(active) = job.as_mut() else {
        return;
    };
    let finished = advance_discovery(out, active);
    WORKER_STACK.note("discovery-advance");
    if finished {
        let done = job.take().expect("job present");
        publish_discovery_outcome(out.ports, &done);
        WORKER_STACK.note("discovery-outcome");
    }
}

#[inline(never)]
fn advance_discovery(out: &mut HaOut<'_>, job: &mut DiscoveryJob) -> bool {
    let gate = if job.workflow.is_some() { Gate::Force } else { Gate::IfChanged };
    for _ in 0..DISCOVERY_BATCH {
        let Some((adapter_id, entity)) = job.pending.pop() else {
            return true;
        };
        match announce(out, adapter_id, entity, gate) {
            Announced::Skipped => {}
            Announced::Published => {
                job.published = job.published.saturating_add(1);
                bump(&out.ports.counters.discovery_published_total);
            }
            Announced::Refused => {
                job.failed = job.failed.saturating_add(1);
                bump(&out.ports.counters.discovery_failed_total);
            }
        }
    }
    job.pending.is_empty()
}

#[inline(never)]
fn publish_discovery_outcome(ports: &MqttWorkerPorts, job: &DiscoveryJob) {
    let Some(workflow) = job.workflow else {
        return;
    };
    publish_event(
        ports,
        workflow,
        dali2rust_contracts::msg::HomeAssistantDiscoveryPublishedEvent {
            entities_published: job.published,
            entities_failed: job.failed,
        },
    );
    publish_event(ports, workflow, terminal_signal(job, workflow));
}

fn terminal_signal(
    job: &DiscoveryJob,
    workflow: u64,
) -> dali2rust_contracts::msg::OperationWorkerSignalEvent {
    if job.failed > 0 {
        OperationWorkerSignalEvent::failed(
            workflow,
            ErrorCode::ExecutionFailed,
            "discovery_publish_refused",
        )
    } else {
        OperationWorkerSignalEvent::succeeded(workflow)
    }
}

#[inline(never)]
pub(super) fn announce(out: &mut HaOut<'_>, adapter_id: u8, entity: Entity, gate: Gate) -> Announced {
    let key = (adapter_id, entity);
    let Some(announcement) = build_discovery(out, adapter_id, entity) else {
        out.session.forget(key);
        return Announced::Skipped;
    };
    if announcement.retracts() {
        return retract(out, key, &announcement, gate);
    }
    let hash = config_hash(&announcement.payload);
    if gate != Gate::Force && out.session.announced.get(&key) == Some(&hash) {
        return Announced::Skipped;
    }
    let delivered = deliver(out, key, &announcement);
    if delivered {
        out.session.announced.insert(key, hash);
    }
    Announced::from(delivered)
}

fn retract(out: &mut HaOut<'_>, key: EntityKey, retraction: &Announcement, gate: Gate) -> Announced {
    if !out.session.forget(key) && gate == Gate::IfAnnounced {
        return Announced::Skipped;
    }
    Announced::from(deliver(out, key, retraction))
}

fn deliver(out: &mut HaOut<'_>, (adapter_id, entity): EntityKey, announcement: &Announcement) -> bool {
    let retain = retain_config(&announcement.payload, out.settings);
    if !out.publish(&announcement.topic, &announcement.payload, retain) {
        return false;
    }
    match entity {
        Entity::Lamp(lamp_id) if announcement.retracts() => clear_lamp_retained(out, adapter_id, lamp_id),
        Entity::Lamp(lamp_id) => {
            publish_lamp_availability(out, adapter_id, lamp_id, announcement.lamp_unreachable)
        }
        _ => true,
    }
}

struct Announcement {
    topic: String,
    payload: String,
    lamp_unreachable: bool,
}

impl Announcement {
    fn config(topic: String, payload: String) -> Self {
        Self { topic, payload, lamp_unreachable: false }
    }

    fn retraction(topic: String) -> Self {
        Self::config(topic, DISCOVERY_RETRACTION.to_string())
    }

    fn retracts(&self) -> bool {
        self.payload == DISCOVERY_RETRACTION
    }
}

fn build_discovery(out: &HaOut<'_>, adapter_id: u8, entity: Entity) -> Option<Announcement> {
    match entity {
        Entity::Lamp(lamp_id) => Some(build_lamp(out, adapter_id, lamp_id)),
        Entity::Group(group_id) => Some(build_group(out, adapter_id, group_id)),
        Entity::Input(short_address, instance_number) => {
            build_input(out, adapter_id, short_address, instance_number)
        }
        Entity::SceneSelect => Some(build_scene_select(out, adapter_id)),
    }
}

#[inline(never)]
fn build_lamp(out: &HaOut<'_>, adapter_id: u8, lamp_id: u8) -> Announcement {
    let topic = out.topics.discovery_topic(LIGHT, &HaTopics::lamp_object_id(adapter_id, lamp_id));
    let Some(lamp) = out.ports.read_port.ha_lamp_view(adapter_id, lamp_id) else {
        return Announcement::retraction(topic);
    };
    Announcement {
        topic,
        payload: lamp_discovery_json(out.topics, &out.device(), adapter_id, &lamp),
        lamp_unreachable: lamp.unreachable,
    }
}

#[inline(never)]
fn build_group(out: &HaOut<'_>, adapter_id: u8, group_id: u8) -> Announcement {
    let topic = out.topics.discovery_topic(LIGHT, &HaTopics::group_object_id(adapter_id, group_id));
    match out.ports.read_port.ha_group_view(adapter_id, group_id) {
        Some(group) => {
            Announcement::config(topic, group_discovery_json(out.topics, &out.device(), adapter_id, &group))
        }
        None => Announcement::retraction(topic),
    }
}

#[inline(never)]
fn build_scene_select(out: &HaOut<'_>, adapter_id: u8) -> Announcement {
    let topic = out.topics.discovery_topic(SELECT, &HaTopics::scene_select_object_id(adapter_id));
    let scenes = out.ports.read_port.ha_scene_views(adapter_id);
    if scenes.is_empty() {
        return Announcement::retraction(topic);
    }
    Announcement::config(topic, scene_select_discovery_json(out.topics, &out.device(), adapter_id, &scenes))
}

#[inline(never)]
fn build_input(out: &HaOut<'_>, adapter_id: u8, short_address: u8, instance_number: u8) -> Option<Announcement> {
    let view = out.ports.read_port.ha_input_view(adapter_id, short_address)?;
    let instance = view.instances.iter().find(|i| i.instance_number == instance_number)?;
    let instance_type = instance.instance_type?;
    let component = input_component(instance_type)?;
    let topic = out.topics.discovery_topic(
        component,
        &HaTopics::input_object_id(adapter_id, short_address, instance_number),
    );
    if !(out.settings.expose_input_devices && view.ha_expose) {
        return Some(Announcement::retraction(topic));
    }
    let (_, payload) = input_instance_discovery_json(
        out.topics,
        &out.device(),
        adapter_id,
        view.name.as_deref(),
        short_address,
        instance_number,
        instance_type,
    )?;
    Some(Announcement::config(topic, payload))
}

const fn input_component(instance_type: u8) -> Option<&'static str> {
    match instance_type {
        1 => Some("event"),
        3 => Some("binary_sensor"),
        2 | 4 => Some("sensor"),
        _ => None,
    }
}

fn retain_config(payload: &str, settings: &HomeAssistantSettingsView) -> bool {
    payload.is_empty() || settings.retain_discovery
}

fn config_hash(payload: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload.hash(&mut hasher);
    hasher.finish()
}
