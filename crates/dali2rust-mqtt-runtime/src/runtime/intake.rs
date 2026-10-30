use std::collections::HashSet;

use dali2rust_api::coalesce::BurstCoalescer;
use dali2rust_bus::worker_counters::bump;
use dali2rust_bus::{BusFrame, BusSubscriberRx};
use dali2rust_contracts::msg::{BusEventPayload, CommandEnvelope};

use crate::runtime::bus_out::refuse_command;
use crate::runtime::discovery::{announce, plan_discovery, DiscoveryJob, Entity, Gate};
use crate::runtime::ha_out::HaOut;
use crate::runtime::rule_traffic::{drain_rule_publishes, RulePublishBudget};
use crate::runtime::state::{
    publish_groups, publish_input_event, publish_runtime_state, publish_scene_select,
    republish_input_configs, republish_scene_select,
};
use crate::runtime::worker::{MqttWorkerPorts, WORKER_STACK};

pub const BRIDGE_COALESCE_FROM: usize = 16;

const GROUP_ID_COUNT: u8 = 16;

dali2rust_contracts::dispatch_bus_commands! {
    pub const MQTT_WORKER_HANDLED_COMMANDS;
    fn dispatch_mqtt_command(
        payload: &dali2rust_contracts::msg::BusCommandPayload,
        job: &mut Option<DiscoveryJob>,
        rule_publishes: &mut Vec<dali2rust_contracts::msg::MqttPublishCommand>,
        adapter_count: u8,
        ports: &MqttWorkerPorts,
        workflow: u64,
    );
    payload = payload;
    ignored = {};
    HomeAssistantDiscoveryPublishCommand(_body) => {
        *job = Some(plan_discovery(ports, adapter_count, Some(workflow)));
    },
    MqttPublishCommand(body) => {
        rule_publishes.push(body.clone());
    },
}

dali2rust_contracts::dispatch_bus_events! {
    pub const MQTT_WORKER_HANDLED_EVENTS;
    fn consume_event(payload: &BusEventPayload, out: &mut HaOut<'_>, touched: &mut HashSet<u8>);
    payload = payload;
    ignored = {};
    RuntimeStateChangedEvent(body) => {
        publish_runtime_state(out, body);
        touched.insert(body.adapter_id);
    },
    DaliInputEventObservedEvent(body) => {
        publish_input_event(out, body);
    },
    InputDeviceChangedEvent(body) => {
        republish_input_configs(out, body.adapter_id, body.short_address);
    },
    PhysicalDeviceChangedEvent(body) => {
        for lamp_id in out.ports.read_port.ha_lamp_ids_for_short(body.adapter_id, body.short_address) {
            announce(out, body.adapter_id, Entity::Lamp(lamp_id), Gate::IfChanged);
        }
        out.session.groups_dirty.insert(body.adapter_id);
    },
    VirtualLampChangedEvent(body) => {
        announce(out, body.adapter_id, Entity::Lamp(body.virtual_lamp_id), Gate::IfChanged);
        out.session.groups_dirty.insert(body.adapter_id);
    },
    GroupChangedEvent(body) => {
        announce(out, body.adapter_id, Entity::Group(body.group_id), Gate::IfChanged);
    },
    GroupMatrixChangedEvent(body) => {
        for group_id in 0..GROUP_ID_COUNT {
            announce(out, body.adapter_id, Entity::Group(group_id), Gate::IfAnnounced);
        }
    },
    SceneChangedEvent(body) => {
        republish_scene_select(out, body.adapter_id);
    },
}

fn frame_tag(frame: &BusFrame) -> &'static str {
    match frame {
        BusFrame::Command(ce) => ce.payload.variant_name(),
        BusFrame::Event(ee) => ee.payload.variant_name(),
        BusFrame::Confirmation(_) => "confirmation",
    }
}

fn intake(ev_rx: &BusSubscriberRx, burst: &mut BurstCoalescer<BusFrame>, ports: &MqttWorkerPorts) {
    while let Ok(frame) = ev_rx.try_recv() {
        let key = if burst.len() >= BRIDGE_COALESCE_FROM {
            match &frame {
                BusFrame::Event(event) => dali2rust_api::ws::coalesce_key(event),
                _ => None,
            }
        } else {
            None
        };
        burst.push(key, frame);
    }
    let backlog = u32::try_from(burst.len()).unwrap_or(u32::MAX);
    dali2rust_bus::worker_counters::raise(&ports.counters.bus_backlog_peak, backlog);
    let superseded = burst.take_superseded();
    if superseded > 0 {
        dali2rust_bus::worker_counters::bump_by(&ports.counters.bus_coalesced_total, superseded);
    }
}

#[inline(never)]
pub(super) fn drain_bus(
    ev_rx: &BusSubscriberRx,
    burst: &mut BurstCoalescer<BusFrame>,
    out: &mut HaOut<'_>,
    job: &mut Option<DiscoveryJob>,
    rule_budget: &mut RulePublishBudget,
) {
    let mut touched: HashSet<u8> = HashSet::new();
    loop {
        intake(ev_rx, burst, out.ports);
        let Some(frame) = burst.pop_front() else {
            break;
        };
        match &frame {
            BusFrame::Command(command) => take_command(command, out, job, rule_budget),
            BusFrame::Event(event) => consume_event(&event.payload, out, &mut touched),
            BusFrame::Confirmation(_) => continue,
        }
        WORKER_STACK.note(frame_tag(&frame));
    }
    fan_out(out, touched);
}

fn take_command(
    command: &CommandEnvelope,
    out: &mut HaOut<'_>,
    job: &mut Option<DiscoveryJob>,
    rule_budget: &mut RulePublishBudget,
) {
    let mut rule_publishes = Vec::new();
    dispatch_mqtt_command(
        &command.payload,
        job,
        &mut rule_publishes,
        out.ports.adapter_count,
        out.ports,
        command.meta.correlation_id,
    );
    drain_rule_publishes(out, rule_budget, rule_publishes);
}

fn fan_out(out: &mut HaOut<'_>, touched: HashSet<u8>) {
    for adapter_id in touched {
        publish_groups(out, adapter_id);
        publish_scene_select(out, adapter_id);
        WORKER_STACK.note("group-fanout");
    }
}

#[inline(never)]
pub(super) fn discard_bus(ev_rx: &BusSubscriberRx, ports: &MqttWorkerPorts) {
    while let Ok(frame) = ev_rx.try_recv() {
        bump(&ports.counters.bus_discarded_total);
        if let BusFrame::Command(command) = &frame {
            refuse_command(ports, command.meta.correlation_id);
        }
    }
}
