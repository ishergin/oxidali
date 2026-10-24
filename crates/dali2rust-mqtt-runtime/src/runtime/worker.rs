use std::sync::mpsc::Receiver;
use std::sync::Arc;

use dali2rust_api::coalesce::BurstCoalescer;
use dali2rust_api::ha::HaTopics;
use dali2rust_bsp::stack_probe::StackLowWater;
use dali2rust_bsp::std_thread_stack;
use dali2rust_bus::{BusFrame, BusId, BusPublisher, BusSubscriberRx};
use dali2rust_domain::registry::{
    HaPublishReadPort, HomeAssistantSecretReadPort, HomeAssistantSettingsApplyWatchPort,
    HomeAssistantSettingsReadPort, HomeAssistantSettingsView,
};
use dali2rust_platform::mqtt::{MqttClient, MqttIncoming, MqttLink};

use crate::counters::MqttCounters;
use crate::runtime::dial::{
    ensure_session, park_after_failed_dial, ready, restart_if_settings_changed, stand_down,
    DialPacer,
};
use crate::runtime::discovery::{advance_job, DiscoveryJob};
use crate::runtime::ha_out::HaOut;
use crate::runtime::inbound::serve_inbound;
use crate::runtime::intake::{discard_bus, drain_bus};
use crate::runtime::rule_topics::{RuleTopicCache, RuleTopicsReadPort};
use crate::runtime::rule_traffic::RulePublishBudget;
use crate::runtime::session::{Session, SettingsCache};

pub struct MqttWorkerPorts {
    pub publisher: BusPublisher,
    pub bus_id: BusId,
    pub correlation: Arc<dali2rust_bus::CorrelationIdAllocator>,
    pub read_port: Arc<dyn HaPublishReadPort>,
    pub settings: Arc<dyn HomeAssistantSettingsReadPort>,
    pub settings_watch: Arc<dyn HomeAssistantSettingsApplyWatchPort>,
    pub secret: Arc<dyn HomeAssistantSecretReadPort>,
    pub counters: Arc<MqttCounters>,
    pub version: &'static str,
    pub adapter_count: u8,
    pub role: Arc<dyn dali2rust_domain::registry::DaliSettingsReadPort>,
    pub rule_topics: Arc<dyn RuleTopicsReadPort>,
}

pub fn spawn_mqtt_worker(
    ev_rx: BusSubscriberRx,
    client: Box<dyn MqttClient>,
    incoming: Receiver<MqttIncoming>,
    link: Arc<MqttLink>,
    ports: MqttWorkerPorts,
) -> std::thread::JoinHandle<()> {
    dali2rust_bsp::esp_thread::spawn_named_stack_in(
        c"mqtt_worker",
        dali2rust_bsp::std_thread_stack::EVENT_WORKER_STACK,
        dali2rust_bsp::esp_thread::StackHome::ExternalOnXip,
        move || run(ev_rx, client, incoming, link, ports),
    )
}

pub(super) static WORKER_STACK: StackLowWater =
    StackLowWater::new("mqtt_worker", std_thread_stack::EVENT_WORKER_STACK);

#[derive(Default)]
struct WorkerLoopState {
    burst: BurstCoalescer<BusFrame>,
    session: Session,
    topics: HaTopics,
    job: Option<DiscoveryJob>,
    rule_budget: RulePublishBudget,
    config: SettingsCache,
    pacer: DialPacer,
    rules: RuleTopicCache,
}

#[inline(never)]
fn new_loop_state() -> Box<WorkerLoopState> {
    Box::new(WorkerLoopState::default())
}

fn run(
    ev_rx: BusSubscriberRx,
    mut client: Box<dyn MqttClient>,
    incoming: Receiver<MqttIncoming>,
    link: Arc<MqttLink>,
    ports: MqttWorkerPorts,
) {
    let mut state = new_loop_state();
    let WorkerLoopState { burst, session, topics, job, rule_budget, config, pacer, rules } = &mut *state;
    WORKER_STACK.note("start");
    loop {
        config.refresh(&ports);
        WORKER_STACK.note("settings-refresh");
        let SettingsCache {
            settings, password, ..
        } = &*config;
        if !ready(settings, ports.role.as_ref()) {
            discard_bus(&ev_rx, &ports);
            stand_down(&mut client, session, topics, &ports, &link, job, rules);
            WORKER_STACK.note("stand-down");
            continue;
        }
        restart_if_settings_changed(&mut client, session, topics, settings, password, &ports);
        if !ensure_session(&mut client, session, topics, job, settings, password, pacer, &ports, rules)
        {
            park_after_failed_dial(&ev_rx, job, pacer, &link, &ports);
            continue;
        }
        WORKER_STACK.note("session");
        let cx = ServeTurn {
            ev_rx: &ev_rx, incoming: &incoming, link: &link, ports: &ports, settings, rules: &mut *rules,
        };
        if !serve_turn(cx, burst, &mut client, session, topics, job, rule_budget) {
            return;
        }
    }
}

pub(super) struct ServeTurn<'a> {
    ev_rx: &'a BusSubscriberRx,
    pub(super) incoming: &'a Receiver<MqttIncoming>,
    pub(super) link: &'a MqttLink,
    pub(super) ports: &'a MqttWorkerPorts,
    settings: &'a HomeAssistantSettingsView,
    pub(super) rules: &'a mut RuleTopicCache,
}

#[inline(never)]
fn serve_turn(
    mut cx: ServeTurn<'_>,
    burst: &mut BurstCoalescer<BusFrame>,
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    job: &mut Option<DiscoveryJob>,
    rule_budget: &mut RulePublishBudget,
) -> bool {
    if !serve_inbound(&mut cx, topics, session) {
        return false;
    }
    WORKER_STACK.note("inbound");
    dali2rust_bus::worker_counters::mirror(
        &cx.ports.counters.commands_dropped_total,
        cx.link.dropped_incoming(),
    );
    serve_bus(&cx, burst, client, session, topics, job, rule_budget);
    true
}

#[inline(never)]
fn serve_bus(
    cx: &ServeTurn<'_>,
    burst: &mut BurstCoalescer<BusFrame>,
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    job: &mut Option<DiscoveryJob>,
    rule_budget: &mut RulePublishBudget,
) {
    let mut out = HaOut { client: client.as_mut(), session, topics, settings: cx.settings, ports: cx.ports };
    drain_bus(cx.ev_rx, burst, &mut out, job, rule_budget);
    advance_job(&mut out, job);
}
