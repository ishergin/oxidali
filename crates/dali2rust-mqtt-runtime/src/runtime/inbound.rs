use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use dali2rust_api::ha::{
    caps_accept, caps_accept_for_group, ha_light_command_to_setpoint, HaCommandTarget, HaTopics,
};
use dali2rust_bus::worker_counters::bump;
use dali2rust_contracts::msg::{DaliRecallSceneCommand, DaliSetTargetStateCommand};
use dali2rust_platform::liveness::monotonic_ms;
use dali2rust_platform::mqtt::MqttIncoming;

use crate::runtime::bus_out::publish_command;
use crate::runtime::rule_traffic::{pace_rule_message, publish_rule_message};
use crate::runtime::session::Session;
use crate::runtime::worker::{MqttWorkerPorts, ServeTurn, WORKER_STACK};

const WAIT_MS: u32 = 100;

#[inline(never)]
pub(super) fn serve_inbound(cx: &mut ServeTurn<'_>, topics: &HaTopics, session: &Session) -> bool {
    let now_ms = monotonic_ms();
    for message in cx.rules.pacer.take_due(now_ms) {
        publish_rule_message(cx.ports, &message);
    }
    let wait = Duration::from_millis(u64::from(cx.rules.pacer.wait_ms(now_ms, WAIT_MS)));
    match cx.incoming.recv_timeout(wait) {
        Ok(message) => {
            route_inbound(cx, topics, session, message, monotonic_ms());
            true
        }
        Err(RecvTimeoutError::Timeout) => true,
        Err(RecvTimeoutError::Disconnected) => false,
    }
}

fn route_inbound(
    cx: &mut ServeTurn<'_>,
    topics: &HaTopics,
    session: &Session,
    message: MqttIncoming,
    now_ms: u32,
) {
    let for_rules = session.rule_topics.follows(&message.topic);
    let for_commands = !for_rules || topics.parse_command_topic(&message.topic).is_some();
    if message.session != session.generation {
        drop_stale(cx, &message, for_rules, for_commands);
        return;
    }
    if for_commands {
        bump(&cx.ports.counters.commands_received_total);
        handle_command(topics, cx.ports, &message);
    }
    if for_rules {
        pace_rule_message(cx.rules, cx.ports, message, now_ms);
    }
}

fn drop_stale(cx: &ServeTurn<'_>, message: &MqttIncoming, for_rules: bool, for_commands: bool) {
    if for_rules && !message.retained {
        bump(&cx.ports.counters.rule_messages_total);
        bump(&cx.ports.counters.rule_messages_lost_total);
    }
    if for_commands {
        cx.link.note_incoming_dropped();
    }
}

fn handle_command(
    topics: &HaTopics,
    ports: &MqttWorkerPorts,
    message: &MqttIncoming,
) {
    let Some(target) = topics.parse_command_topic(&message.topic) else {
        bump(&ports.counters.commands_unroutable_total);
        return;
    };
    let routed = match target {
        HaCommandTarget::VirtualLamp {
            adapter_id,
            virtual_lamp_id,
        } => lamp_command(ports, adapter_id, virtual_lamp_id, &message.payload),
        HaCommandTarget::Group {
            adapter_id,
            group_id,
        } => group_command(ports, adapter_id, group_id, &message.payload),
        HaCommandTarget::SceneSelect { adapter_id } => {
            scene_command(ports, adapter_id, &message.payload)
        }
    };
    if !routed {
        bump(&ports.counters.commands_unroutable_total);
    }
}

fn lamp_command(
    ports: &MqttWorkerPorts,
    adapter_id: u8,
    virtual_lamp_id: u8,
    payload: &[u8],
) -> bool {
    let Some(lamp) = ports.read_port.ha_lamp_view(adapter_id, virtual_lamp_id) else {
        return false;
    };
    let caps = lamp.capabilities;
    WORKER_STACK.note("inbound:lamp-view");
    let Ok(setpoint) = ha_light_command_to_setpoint(payload, |m| caps_accept(&caps, m)) else {
        return false;
    };
    WORKER_STACK.note("inbound:parsed");
    publish_command(
        ports,
        DaliSetTargetStateCommand::for_virtual_lamp(adapter_id, virtual_lamp_id, &setpoint),
    );
    WORKER_STACK.note("inbound:published");
    true
}

fn group_command(ports: &MqttWorkerPorts, adapter_id: u8, group_id: u8, payload: &[u8]) -> bool {
    let Some(group) = ports.read_port.ha_group_view(adapter_id, group_id) else {
        return false;
    };
    let caps = group.capabilities;
    let Ok(setpoint) = ha_light_command_to_setpoint(payload, |m| caps_accept_for_group(&caps, m))
    else {
        return false;
    };
    publish_command(
        ports,
        DaliSetTargetStateCommand::for_group(adapter_id, group_id, &setpoint),
    );
    true
}

fn scene_command(ports: &MqttWorkerPorts, adapter_id: u8, payload: &[u8]) -> bool {
    let wanted = String::from_utf8_lossy(payload).trim().to_string();
    if wanted == dali2rust_api::ha::SCENE_SELECT_NONE {
        return true;
    }
    let Some(scene) = ports
        .read_port
        .ha_scene_views(adapter_id)
        .into_iter()
        .find(|s| dali2rust_api::ha::scene_option_label(s) == wanted)
    else {
        return false;
    };
    publish_command(
        ports,
        DaliRecallSceneCommand::broadcast(adapter_id, scene.scene_id),
    );
    true
}
