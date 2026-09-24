use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_api::ha::{
    caps_accept, caps_accept_for_group, group_discovery_json, group_state_json,
    ha_light_command_to_setpoint,
    input_event_state_payload, input_instance_discovery_json, lamp_discovery_json,
    light_state_json, scene_select_discovery_json, scene_select_state,
    HaCommandTarget, HaDevice, HaTopics, DISCOVERY_RETRACTION,
};
use dali2rust_bsp::stack_probe::StackLowWater;
use dali2rust_bsp::std_thread_stack;
use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, BusSubscriberRx, PublishResult};
use dali2rust_contracts::msg::{
    BusEventPayload, DaliRecallSceneCommand, DaliSetTargetStateCommand, Origin,
};
use dali2rust_contracts::SOURCE_ID_UNSPECIFIED;
use dali2rust_domain::registry::{
    HaPublishReadPort, HomeAssistantSecretReadPort, HomeAssistantSettingsApplyWatchPort,
    HomeAssistantSettingsReadPort, HomeAssistantSettingsView,
};
use dali2rust_platform::mqtt::{
    MqttClient, MqttConnectionState, MqttIncoming, MqttLink, MqttQos,
};

use dali2rust_api::coalesce::BurstCoalescer;
use crate::counters::MqttCounters;
use crate::runtime::session::{
    announce_and_subscribe, announce_offline, qos_of, session_config, topics_of,
};

const WAIT: Duration = Duration::from_millis(100);
const PARK: Duration = Duration::from_millis(500);



const COALESCE_FROM: usize = 16;

const REDIAL_BACKOFF: &[Duration] = &[Duration::from_secs(2), Duration::from_secs(8)];

const DISCOVERY_BATCH: usize = 4;

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
}

#[derive(Default)]
struct Session {
    generation: u32,
    subscriptions_expected: u32,
    lamp_config_hashes: HashMap<(u8, u8), u64>,
    input_config_hashes: HashMap<(u8, u8, u8), u64>,
    lamp_availability: HashMap<(u8, u8), bool>,
    group_config_hashes: HashMap<(u8, u8), u64>,
    groups_dirty: HashSet<u8>,
    announced_selects: HashSet<u8>,
    dialled_with: u64,
}

impl Session {
    fn begin(&mut self, generation: u32, dialled_with: u64) {
        let Self {
            generation: gen_slot,
            subscriptions_expected,
            dialled_with: dialled_slot,
            lamp_config_hashes,
            input_config_hashes,
            lamp_availability,
            group_config_hashes,
            groups_dirty,
            announced_selects,
        } = self;
        *gen_slot = generation;
        *subscriptions_expected = 0;
        *dialled_slot = dialled_with;
        lamp_config_hashes.clear();
        input_config_hashes.clear();
        lamp_availability.clear();
        group_config_hashes.clear();
        groups_dirty.clear();
        announced_selects.clear();
    }
}

struct SettingsCache {
    revision: u32,
    settings: HomeAssistantSettingsView,
    password: String,
}

impl Default for SettingsCache {
    fn default() -> Self {
        Self {
            revision: u32::MAX,
            settings: HomeAssistantSettingsView::default(),
            password: String::new(),
        }
    }
}

impl SettingsCache {
    fn refresh(&mut self, ports: &MqttWorkerPorts) {
        let revision = ports.settings_watch.home_assistant_settings_applied_load();
        if revision == self.revision {
            return;
        }
        self.settings = ports.settings.home_assistant_settings_view();
        self.password = ports.secret.home_assistant_broker_password();
        self.revision = revision;
    }
}

#[derive(Default)]
struct DialPacer {
    last_attempt: Option<Instant>,
    failures: u32,
    dialled: u64,
}

fn session_settings_fingerprint(settings: &HomeAssistantSettingsView, password: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    settings.broker_host.hash(&mut hasher);
    settings.broker_port.hash(&mut hasher);
    settings.broker_username.hash(&mut hasher);
    settings.controller_id.hash(&mut hasher);
    settings.discovery_prefix.hash(&mut hasher);
    settings.state_topic_prefix.hash(&mut hasher);
    password.hash(&mut hasher);
    hasher.finish()
}

impl DialPacer {
    fn defer_redial(&self, settings: &HomeAssistantSettingsView, password: &str) -> bool {
        let Some(at) = self.last_attempt else {
            return false;
        };
        if self.failures == 0 {
            return false;
        }
        if self.dialled != session_settings_fingerprint(settings, password) {
            return false;
        }
        let step = (self.failures.saturating_sub(1) as usize).min(REDIAL_BACKOFF.len() - 1);
        at.elapsed() < REDIAL_BACKOFF[step]
    }

    fn note_attempt(&mut self, settings: &HomeAssistantSettingsView, password: &str) {
        let dialled = session_settings_fingerprint(settings, password);
        if self.dialled != dialled {
            self.failures = 0;
            self.dialled = dialled;
        }
        self.failures = self.failures.saturating_add(1);
        self.last_attempt = Some(Instant::now());
    }

    fn established(&mut self) {
        self.failures = 0;
    }
}

struct DiscoveryJob {
    pending: Vec<(u8, JobEntity)>,
    published: u16,
    failed: u16,
    workflow: Option<u64>,
}

enum JobEntity {
    Lamp(u8),
    Group(u8),
    InputInstance(u8, u8),
    SceneSelect,
    Retract(&'static str, String),
    RetractLamp(u8),
}

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
        let settings = ports.settings.home_assistant_settings_view();
        *job = Some(plan_discovery(ports, &settings, adapter_count, Some(workflow)));
    },
    MqttPublishCommand(body) => {
        rule_publishes.push(body.clone());
    },
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

static WORKER_STACK: StackLowWater =
    StackLowWater::new("mqtt_worker", std_thread_stack::EVENT_WORKER_STACK);

fn frame_tag(frame: &BusFrame) -> &'static str {
    match frame {
        BusFrame::Command(ce) => ce.payload.variant_name(),
        BusFrame::Event(ee) => ee.payload.variant_name(),
        BusFrame::Confirmation(_) => "confirmation",
    }
}

#[derive(Default)]
struct WorkerLoopState {
    burst: BurstCoalescer<BusFrame>,
    session: Session,
    topics: HaTopics,
    job: Option<DiscoveryJob>,
    rule_budget: RulePublishBudget,
    config: SettingsCache,
    pacer: DialPacer,
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
    let WorkerLoopState { burst, session, topics, job, rule_budget, config, pacer } = &mut *state;
    WORKER_STACK.note("start");
    loop {
        config.refresh(&ports);
        WORKER_STACK.note("settings-refresh");
        let SettingsCache {
            settings, password, ..
        } = &*config;
        if !ready(settings, ports.role.as_ref()) {
            discard_bus(&ev_rx, &ports);
            stand_down(&mut client, session, topics, &ports, &link, job);
            WORKER_STACK.note("stand-down");
            continue;
        }
        restart_if_settings_changed(&mut client, session, topics, settings, password, &ports);
        if !ensure_session(&mut client, session, topics, job, settings, password, pacer, &ports)
        {
            park_after_failed_dial(&ev_rx, job, pacer, &link, &ports);
            continue;
        }
        WORKER_STACK.note("session");
        if !serve_turn(
            ServeTurn {
                ev_rx: &ev_rx,
                incoming: &incoming,
                link: &link,
                ports: &ports,
                settings,
            },
            burst,
            &mut client,
            session,
            topics,
            job,
            rule_budget,
        ) {
            return;
        }
    }
}

#[inline(never)]
fn park_after_failed_dial(
    ev_rx: &BusSubscriberRx,
    job: &mut Option<DiscoveryJob>,
    pacer: &DialPacer,
    link: &Arc<MqttLink>,
    ports: &MqttWorkerPorts,
) {
    fail_job_after_dial_streak(job, pacer, link, ports);
    discard_bus(ev_rx, ports);
    WORKER_STACK.note("dial");
    link.wait_for_change(PARK);
}

struct ServeTurn<'a> {
    ev_rx: &'a BusSubscriberRx,
    incoming: &'a Receiver<MqttIncoming>,
    link: &'a MqttLink,
    ports: &'a MqttWorkerPorts,
    settings: &'a HomeAssistantSettingsView,
}

#[inline(never)]
fn serve_turn(
    cx: ServeTurn<'_>,
    burst: &mut BurstCoalescer<BusFrame>,
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    job: &mut Option<DiscoveryJob>,
    rule_budget: &mut RulePublishBudget,
) -> bool {
    if !service_inbound(cx.incoming, topics, cx.ports) {
        return false;
    }
    WORKER_STACK.note("inbound");
    dali2rust_bus::worker_counters::mirror(
        &cx.ports.counters.commands_dropped_total,
        cx.link.dropped_incoming(),
    );
    drain_bus(cx.ev_rx, burst, client, session, topics, cx.settings, cx.ports, job, rule_budget);
    if let Some(active) = job.as_mut() {
        let finished = advance_discovery(client, session, active, topics, cx.settings, cx.ports);
        WORKER_STACK.note("discovery-advance");
        if finished {
            let done = job.take().expect("job present");
            publish_discovery_outcome(cx.ports, &done);
            WORKER_STACK.note("discovery-outcome");
        }
    }
    true
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
    use dali2rust_contracts::msg::{ErrorCode, OperationWorkerSignalEvent};
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

pub const MQTT_WORKER_REQUIRED_EVENTS: &[&str] = &[
    "HomeAssistantDiscoveryPublishedEvent",
    "OperationWorkerSignalEvent",
];

fn publish_event<P>(ports: &MqttWorkerPorts, correlation_id: u64, payload: P)
where
    P: Into<dali2rust_contracts::msg::BusEventPayload>,
{
    let envelope = dali2rust_contracts::bus::event_envelope(
        SOURCE_ID_UNSPECIFIED,
        correlation_id,
        ports.bus_id.0,
        Some(Origin::Mqtt),
        payload,
    );
    dali2rust_bus::publish_required_counted(
        &ports.publisher,
        BusChannel::Events,
        BusFrame::event(envelope),
        &dali2rust_bus::REQUIRED_PUBLISH_BACKOFF_MS,
        dali2rust_bus::REQUIRED_PUBLISH_UNCAPPED,
        "mqtt-bridge",
        dali2rust_bus::RequiredPublishCounters::new(
            &ports.counters.terminal_event_publish_retried_total,
            &ports.counters.terminal_event_publish_failed_total,
        ),
    );
}

#[inline(never)]
fn discard_bus(ev_rx: &BusSubscriberRx, ports: &MqttWorkerPorts) {
    while let Ok(frame) = ev_rx.try_recv() {
        bump(&ports.counters.bus_discarded_total);
        if let BusFrame::Command(command) = &frame {
            refuse_command(ports, command.meta.correlation_id);
        }
    }
}

fn refuse_command(ports: &MqttWorkerPorts, workflow: u64) {
    publish_event(
        ports,
        workflow,
        dali2rust_contracts::msg::OperationWorkerSignalEvent::failed(
            workflow,
            dali2rust_contracts::msg::ErrorCode::ExecutionFailed,
            "ha_bridge_unavailable",
        ),
    );
}

fn plan_input_discovery(
    pending: &mut Vec<(u8, JobEntity)>,
    ports: &MqttWorkerPorts,
    settings: &HomeAssistantSettingsView,
    adapter_id: u8,
) {
    for view in ports.read_port.ha_input_views(adapter_id) {
        let announce = settings.expose_input_devices && view.ha_expose;
        for instance in &view.instances {
            let Some(component) = instance.instance_type.and_then(input_component) else {
                continue;
            };
            if announce {
                pending.push((
                    adapter_id,
                    JobEntity::InputInstance(view.short_address, instance.instance_number),
                ));
            } else {
                pending.push((
                    adapter_id,
                    JobEntity::Retract(
                        component,
                        HaTopics::input_object_id(
                            adapter_id,
                            view.short_address,
                            instance.instance_number,
                        ),
                    ),
                ));
            }
        }
    }
}

#[inline(never)]
fn plan_discovery(
    ports: &MqttWorkerPorts,
    settings: &HomeAssistantSettingsView,
    adapter_count: u8,
    workflow: Option<u64>,
) -> DiscoveryJob {
    let mut pending = Vec::new();
    for adapter_id in 0..adapter_count {
        plan_input_discovery(&mut pending, ports, settings, adapter_id);
        for lamp in ports.read_port.ha_lamp_views(adapter_id) {
            pending.push((adapter_id, JobEntity::Lamp(lamp.virtual_lamp_id)));
        }
        for group in ports.read_port.ha_group_views(adapter_id) {
            pending.push((adapter_id, JobEntity::Group(group.group_id)));
        }
        if ports.read_port.ha_scene_views(adapter_id).is_empty() {
            pending.push((
                adapter_id,
                JobEntity::Retract("select", HaTopics::scene_select_object_id(adapter_id)),
            ));
        } else {
            pending.push((adapter_id, JobEntity::SceneSelect));
        }
        for id in ports.read_port.ha_retracted_lamp_ids(adapter_id) {
            pending.push((adapter_id, JobEntity::RetractLamp(id)));
        }
        for id in ports.read_port.ha_retracted_group_ids(adapter_id) {
            pending.push((
                adapter_id,
                JobEntity::Retract("light", HaTopics::group_object_id(adapter_id, id)),
            ));
        }
    }
    pending.reverse();
    DiscoveryJob {
        pending,
        published: 0,
        failed: 0,
        workflow,
    }
}

fn build_scene_select_discovery(
    ports: &MqttWorkerPorts,
    topics: &HaTopics,
    device: &HaDevice<'_>,
    adapter_id: u8,
) -> Option<(String, String)> {
    let scenes = ports.read_port.ha_scene_views(adapter_id);
    let topic = topics.discovery_topic("select", &HaTopics::scene_select_object_id(adapter_id));
    if scenes.is_empty() {
        return Some((topic, DISCOVERY_RETRACTION.to_string()));
    }
    Some((
        topic,
        scene_select_discovery_json(topics, device, adapter_id, &scenes),
    ))
}

struct Announcement {
    topic: String,
    payload: String,
    lamp_unreachable: Option<bool>,
}

impl Announcement {
    fn plain((topic, payload): (String, String)) -> Self {
        Self { topic, payload, lamp_unreachable: None }
    }
}

#[inline(never)]
fn build_discovery(
    ports: &MqttWorkerPorts,
    topics: &HaTopics,
    device: &HaDevice<'_>,
    settings: &HomeAssistantSettingsView,
    adapter_id: u8,
    entity: &JobEntity,
) -> Option<Announcement> {
    match entity {
        JobEntity::InputInstance(short_address, instance_number) => build_input_discovery(
            ports,
            topics,
            device,
            settings,
            adapter_id,
            *short_address,
            *instance_number,
        )
        .map(Announcement::plain),
        JobEntity::Lamp(id) => ports.read_port.ha_lamp_view(adapter_id, *id).map(|lamp| {
            Announcement {
                topic: topics.discovery_topic("light", &HaTopics::lamp_object_id(adapter_id, *id)),
                payload: lamp_discovery_json(topics, device, adapter_id, &lamp),
                lamp_unreachable: Some(lamp.unreachable),
            }
        }),
        JobEntity::Group(id) => ports
            .read_port
            .ha_group_view(adapter_id, *id)
            .map(|group| {
                Announcement::plain((
                    topics.discovery_topic("light", &HaTopics::group_object_id(adapter_id, *id)),
                    group_discovery_json(topics, device, adapter_id, &group),
                ))
            }),
        JobEntity::SceneSelect => {
            build_scene_select_discovery(ports, topics, device, adapter_id).map(Announcement::plain)
        }
        JobEntity::Retract(component, object_id) => Some(Announcement::plain((
            topics.discovery_topic(component, object_id),
            DISCOVERY_RETRACTION.to_string(),
        ))),
        JobEntity::RetractLamp(id) => Some(Announcement::plain((
            topics.discovery_topic("light", &HaTopics::lamp_object_id(adapter_id, *id)),
            DISCOVERY_RETRACTION.to_string(),
        ))),
    }
}

#[allow(clippy::too_many_arguments, reason = "the session's caches are threaded, not rebuilt")]
#[inline(never)]
fn announce_one(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    adapter_id: u8,
    entity: &JobEntity,
    announcement: Announcement,
) -> bool {
    let Announcement { topic, payload, lamp_unreachable } = announcement;
    if !publish(client, ports, &topic, &payload, qos_of(settings), retain_config(&payload, settings))
    {
        return false;
    }
    let paired = match entity {
        JobEntity::Lamp(lamp_id) => publish_lamp_availability(
            client,
            session,
            ports,
            topics,
            settings,
            adapter_id,
            *lamp_id,
            lamp_unreachable.unwrap_or(false),
        ),
        JobEntity::RetractLamp(lamp_id) => {
            clear_lamp_retained(client, session, ports, topics, settings, adapter_id, *lamp_id)
        }
        _ => true,
    };
    if paired {
        record_announcement(session, adapter_id, entity, &payload);
    }
    paired
}

fn announced_this_session(
    session: &Session,
    adapter_id: u8,
    entity: &JobEntity,
    payload: &str,
) -> bool {
    let hash = config_hash(payload);
    match entity {
        JobEntity::Lamp(id) => session.lamp_config_hashes.get(&(adapter_id, *id)) == Some(&hash),
        JobEntity::Group(id) => session.group_config_hashes.get(&(adapter_id, *id)) == Some(&hash),
        JobEntity::InputInstance(short_address, instance_number) => {
            session
                .input_config_hashes
                .get(&(adapter_id, *short_address, *instance_number))
                == Some(&hash)
        }
        JobEntity::SceneSelect => session.announced_selects.contains(&adapter_id),
        JobEntity::Retract(..) | JobEntity::RetractLamp(_) => false,
    }
}

fn record_announcement(
    session: &mut Session,
    adapter_id: u8,
    entity: &JobEntity,
    payload: &str,
) {
    let hash = config_hash(payload);
    match entity {
        JobEntity::Lamp(id) => {
            session.lamp_config_hashes.insert((adapter_id, *id), hash);
        }
        JobEntity::Group(id) => {
            session.group_config_hashes.insert((adapter_id, *id), hash);
        }
        JobEntity::InputInstance(short_address, instance_number) => {
            session
                .input_config_hashes
                .insert((adapter_id, *short_address, *instance_number), hash);
        }
        JobEntity::SceneSelect => {
            session.announced_selects.insert(adapter_id);
        }
        JobEntity::RetractLamp(id) => {
            session.lamp_config_hashes.remove(&(adapter_id, *id));
        }
        JobEntity::Retract(..) => {}
    }
}

#[inline(never)]
fn advance_discovery(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    job: &mut DiscoveryJob,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
) -> bool {
    let device = ha_device(settings, ports);
    for _ in 0..DISCOVERY_BATCH {
        let Some((adapter_id, entity)) = job.pending.pop() else {
            return true;
        };
        let Some(announcement) = build_discovery(ports, topics, &device, settings, adapter_id, &entity)
        else {
            continue;
        };
        if job.workflow.is_none()
            && announced_this_session(session, adapter_id, &entity, &announcement.payload)
        {
            continue;
        }
        let published = announce_one(
            client, session, topics, settings, ports, adapter_id, &entity, announcement,
        );
        if published {
            job.published = job.published.saturating_add(1);
            bump(&ports.counters.discovery_published_total);
        } else {
            job.failed = job.failed.saturating_add(1);
            bump(&ports.counters.discovery_failed_total);
        }
    }
    job.pending.is_empty()
}

fn ready(
    settings: &HomeAssistantSettingsView,
    role: &dyn dali2rust_domain::registry::DaliSettingsReadPort,
) -> bool {
    settings.enabled
        && !settings.broker_host.is_empty()
        && role.dali_settings_view().application_active
}

fn retain_config(payload: &str, settings: &HomeAssistantSettingsView) -> bool {
    payload.is_empty() || settings.retain_discovery
}

#[inline(never)]
fn stand_down(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    ports: &MqttWorkerPorts,
    link: &Arc<MqttLink>,
    job: &mut Option<DiscoveryJob>,
) {
    if let Some(workflow) = job.take().and_then(|stalled| stalled.workflow) {
        refuse_command(ports, workflow);
    }
    ports.counters.set_connected(false);
    if link.is_connected() {
        if session.generation != 0 {
            announce_offline(client.as_mut(), topics);
        }
        client.disconnect();
        *session = Session::default();
    }
    link.wait_for_change(PARK);
}


const STALLED_JOB_DIAL_STREAK: u32 = 2;

#[inline(never)]
fn fail_job_after_dial_streak(
    job: &mut Option<DiscoveryJob>,
    pacer: &DialPacer,
    link: &Arc<MqttLink>,
    ports: &MqttWorkerPorts,
) {
    if pacer.failures < STALLED_JOB_DIAL_STREAK
        || link.state() != MqttConnectionState::Disconnected
    {
        return;
    }
    if let Some(workflow) = job.take().and_then(|stalled| stalled.workflow) {
        refuse_command(ports, workflow);
    }
}

#[inline(never)]
fn service_inbound(
    incoming: &Receiver<MqttIncoming>,
    topics: &HaTopics,
    ports: &MqttWorkerPorts,
) -> bool {
    match incoming.recv_timeout(WAIT) {
        Ok(message) => {
            bump(&ports.counters.commands_received_total);
            handle_command(topics, ports, &message);
            true
        }
        Err(RecvTimeoutError::Timeout) => true,
        Err(RecvTimeoutError::Disconnected) => false,
    }
}

#[inline(never)]
fn restart_if_settings_changed(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    password: &str,
    ports: &MqttWorkerPorts,
) {
    if !client.link().is_connected() {
        return;
    }
    if session.generation == 0 {
        return;
    }
    if session.dialled_with == session_settings_fingerprint(settings, password) {
        return;
    }
    announce_offline(client.as_mut(), topics);
    client.disconnect();
    ports.counters.set_connected(false);
    *session = Session::default();
}

#[allow(clippy::too_many_arguments, reason = "the loop's state is threaded, not rebuilt")]
#[inline(never)]
fn ensure_session(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &mut HaTopics,
    job: &mut Option<DiscoveryJob>,
    settings: &HomeAssistantSettingsView,
    password: &str,
    pacer: &mut DialPacer,
    ports: &MqttWorkerPorts,
) -> bool {
    let link = client.link();
    if !link.is_connected() {
        ports.counters.set_connected(false);
        if !dial(client, &link, settings, password, pacer, ports) {
            return false;
        }
    }
    pacer.established();
    if link.session_generation() != session.generation {
        let announced = topics_of(settings);
        if let Err(e) = announce_and_subscribe(client.as_mut(), &announced) {
            log::warn!("mqtt: could not announce on a fresh session: {e:?}");
            announce_offline(client.as_mut(), &announced);
            return false;
        }
        let subscriptions = u32::try_from(announced.command_subscriptions().len()).unwrap_or(u32::MAX);
        *topics = announced;
        session.begin(
            link.session_generation(),
            session_settings_fingerprint(settings, password),
        );
        session.subscriptions_expected = subscriptions;
        plan_session_reannounce(job, ports, settings);
    }
    if !ports.counters.is_connected()
        && link.subscriptions_acked() >= session.subscriptions_expected
    {
        ports.counters.set_connected(true);
    }
    true
}

#[inline(never)]
fn plan_session_reannounce(
    job: &mut Option<DiscoveryJob>,
    ports: &MqttWorkerPorts,
    settings: &HomeAssistantSettingsView,
) {
    let carried = job.take().and_then(|interrupted| interrupted.workflow);
    *job = Some(plan_discovery(ports, settings, ports.adapter_count, carried));
}

fn dial(
    client: &mut Box<dyn MqttClient>,
    link: &Arc<MqttLink>,
    settings: &HomeAssistantSettingsView,
    password: &str,
    pacer: &mut DialPacer,
    ports: &MqttWorkerPorts,
) -> bool {
    if link.state() == MqttConnectionState::Connecting {
        return false;
    }
    if pacer.defer_redial(settings, password) {
        return false;
    }
    pacer.note_attempt(settings, password);
    let dial_topics = topics_of(settings);
    let config = session_config(settings, password.to_string(), &dial_topics);
    if let Err(e) = client.connect(&config) {
        log::warn!("mqtt: connect refused: {e:?}");
        return false;
    }
    bump(&ports.counters.connects_total);
    link.is_connected()
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

use dali2rust_bus::worker_counters::bump;

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

fn publish_command<P>(ports: &MqttWorkerPorts, payload: P)
where
    P: Into<dali2rust_contracts::msg::BusCommandPayload>,
{
    let corr = ports.correlation.next_id();
    let envelope = dali2rust_contracts::bus::command_envelope(
        SOURCE_ID_UNSPECIFIED,
        corr,
        ports.bus_id.0,
        Some(Origin::Mqtt),
        payload,
    );
    let result = ports
        .publisher
        .try_publish(BusChannel::Commands, BusFrame::command(envelope));
    if result != PublishResult::Queued {
        log::warn!("mqtt: command dropped at bus ingress: {result:?}");
        bump(&ports.counters.commands_ingress_rejected_total);
    }
}

fn intake(ev_rx: &BusSubscriberRx, burst: &mut BurstCoalescer<BusFrame>, ports: &MqttWorkerPorts) {
    while let Ok(frame) = ev_rx.try_recv() {
        let key = if burst.len() >= COALESCE_FROM {
            match &frame {
                BusFrame::Event(event) => dali2rust_api::ws::coalesce_key(event),
                _ => None,
            }
        } else {
            None
        };
        burst.push(key, frame);
    }
    let superseded = burst.take_superseded();
    if superseded > 0 {
        dali2rust_bus::worker_counters::bump_by(&ports.counters.bus_coalesced_total, superseded);
    }
}

#[allow(clippy::too_many_arguments, reason = "one call site; the cache is threaded, not rebuilt")]
#[inline(never)]
fn drain_bus(
    ev_rx: &BusSubscriberRx,
    burst: &mut BurstCoalescer<BusFrame>,
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    job: &mut Option<DiscoveryJob>,
    rule_budget: &mut RulePublishBudget,
) {
    let mut touched: HashSet<u8> = HashSet::new();
    loop {
        intake(ev_rx, burst, ports);
        let Some(frame) = burst.pop_front() else {
            break;
        };
        if let BusFrame::Command(command) = &frame {
            let mut rule_publishes = Vec::new();
            dispatch_mqtt_command(
                &command.payload,
                job,
                &mut rule_publishes,
                ports.adapter_count,
                ports,
                command.meta.correlation_id,
            );
            drain_rule_publishes(client, ports, rule_budget, rule_publishes);
            WORKER_STACK.note(frame_tag(&frame));
            continue;
        }
        let BusFrame::Event(event) = &frame else {
            continue;
        };
        consume_event(&event.payload, client, session, topics, settings, ports, &mut touched);
        WORKER_STACK.note(frame_tag(&frame));
    }
    for adapter_id in touched {
        publish_groups(client, session, topics, settings, ports, adapter_id);
        publish_scene_select(client, session, topics, settings, ports, adapter_id);
        WORKER_STACK.note("group-fanout");
    }
}

dali2rust_contracts::dispatch_bus_events! {
    pub const MQTT_WORKER_HANDLED_EVENTS;
    fn consume_event(
        payload: &BusEventPayload,
        client: &mut Box<dyn MqttClient>,
        session: &mut Session,
        topics: &HaTopics,
        settings: &HomeAssistantSettingsView,
        ports: &MqttWorkerPorts,
        touched: &mut HashSet<u8>,
    );
    payload = payload;
    ignored = {};
    RuntimeStateChangedEvent(body) => {
        publish_runtime_state(client, session, topics, settings, ports, body);
        touched.insert(body.adapter_id);
    },
    DaliInputEventObservedEvent(body) => {
        publish_input_event(client, session, topics, settings, ports, body);
    },
    InputDeviceChangedEvent(body) => {
        republish_input_configs(
            client, session, topics, settings, ports, body.adapter_id, body.short_address,
        );
    },
    PhysicalDeviceChangedEvent(body) => {
        for lamp_id in ports
            .read_port
            .ha_lamp_ids_for_short(body.adapter_id, body.short_address)
        {
            republish_lamp_config(
                client, session, topics, settings, ports, body.adapter_id, lamp_id,
                RetractGate::Always,
            );
        }
        session.groups_dirty.insert(body.adapter_id);
    },
    VirtualLampChangedEvent(body) => {
        republish_lamp_config(
            client,
            session,
            topics,
            settings,
            ports,
            body.adapter_id,
            body.virtual_lamp_id,
            RetractGate::Always,
        );
        session.groups_dirty.insert(body.adapter_id);
    },
    GroupChangedEvent(body) => {
        republish_group_config(
            client, session, topics, settings, ports, body.adapter_id, body.group_id,
            RetractGate::Always,
        );
    },
    GroupMatrixChangedEvent(body) => {
        for group_id in 0..GROUP_ID_COUNT {
            republish_group_config(
                client, session, topics, settings, ports, body.adapter_id, group_id,
                RetractGate::IfAnnounced,
            );
        }
    },
    SceneChangedEvent(body) => {
        republish_scene_select(client, session, topics, settings, ports, body.adapter_id);
    },
}

const GROUP_ID_COUNT: u8 = 16;

#[inline(never)]
fn republish_scene_select(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    adapter_id: u8,
) {
    session.announced_selects.remove(&adapter_id);
    if ports.read_port.ha_scene_views(adapter_id).is_empty() {
        let object_id = HaTopics::scene_select_object_id(adapter_id);
        publish(
            client,
            ports,
            &topics.discovery_topic("select", &object_id),
            DISCOVERY_RETRACTION,
            qos_of(settings),
            retain_config(DISCOVERY_RETRACTION, settings),
        );
        return;
    }
    publish_scene_select(client, session, topics, settings, ports, adapter_id);
}

fn publish_groups(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    adapter_id: u8,
) {
    let recheck = session.groups_dirty.remove(&adapter_id);
    for group in ports.read_port.ha_group_views(adapter_id) {
        if recheck || !session.group_config_hashes.contains_key(&(adapter_id, group.group_id)) {
            announce_group(client, session, topics, settings, ports, adapter_id, &group);
        }
        publish(
            client,
            ports,
            &topics.group_state_topic(adapter_id, group.group_id),
            &group_state_json(&group.state),
            qos_of(settings),
            settings.retain_state,
        );
    }
}

#[allow(clippy::too_many_arguments, reason = "the cache is threaded, not rebuilt per call")]
fn announce_group(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    adapter_id: u8,
    group: &dali2rust_domain::registry::HaGroupView,
) {
    let payload = group_discovery_json(topics, &ha_device(settings, ports), adapter_id, group);
    let hash = config_hash(&payload);
    let key = (adapter_id, group.group_id);
    if session.group_config_hashes.get(&key) == Some(&hash) {
        return;
    }
    let object_id = HaTopics::group_object_id(adapter_id, group.group_id);
    let topic = topics.discovery_topic("light", &object_id);
    if publish(client, ports, &topic, &payload, qos_of(settings), retain_config(&payload, settings))
    {
        session.group_config_hashes.insert(key, hash);
    }
}

#[allow(clippy::too_many_arguments, reason = "the cache is threaded, not rebuilt per call")]
#[inline(never)]
fn republish_group_config(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    adapter_id: u8,
    group_id: u8,
    retract: RetractGate,
) {
    let Some(group) = ports.read_port.ha_group_view(adapter_id, group_id) else {
        let announced_here = session.group_config_hashes.remove(&(adapter_id, group_id)).is_some();
        if !announced_here && retract == RetractGate::IfAnnounced {
            return;
        }
        let object_id = HaTopics::group_object_id(adapter_id, group_id);
        publish(
            client,
            ports,
            &topics.discovery_topic("light", &object_id),
            DISCOVERY_RETRACTION,
            qos_of(settings),
            retain_config(DISCOVERY_RETRACTION, settings),
        );
        return;
    };
    announce_group(client, session, topics, settings, ports, adapter_id, &group);
}

fn publish_scene_select(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    adapter_id: u8,
) {
    let scenes = ports.read_port.ha_scene_views(adapter_id);
    if scenes.is_empty() {
        return;
    }
    if !session.announced_selects.contains(&adapter_id) {
        let object_id = HaTopics::scene_select_object_id(adapter_id);
        if publish(
            client,
            ports,
            &topics.discovery_topic("select", &object_id),
            &scene_select_discovery_json(topics, &ha_device(settings, ports), adapter_id, &scenes),
            qos_of(settings),
            settings.retain_discovery,
        ) {
            session.announced_selects.insert(adapter_id);
        }
    }
    let active = ports.read_port.ha_active_scene(adapter_id);
    publish(
        client,
        ports,
        &topics.scene_select_state_topic(adapter_id),
        &scene_select_state(active, &scenes),
        qos_of(settings),
        settings.retain_state,
    );
}

fn ha_device<'a>(
    settings: &'a HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
) -> HaDevice<'a> {
    HaDevice {
        controller_id: &settings.controller_id,
        version: ports.version,
    }
}

#[allow(clippy::too_many_arguments, reason = "the cache is threaded, not rebuilt per call")]
fn publish_runtime_state(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    body: &dali2rust_contracts::msg::RuntimeStateChangedEvent,
) {
    if let Some(lamp_id) = body.virtual_lamp_id {
        publish_lamp(client, session, topics, settings, ports, body, lamp_id);
        return;
    }
    let Some(short_address) = body.short_address else {
        return;
    };
    for lamp_id in ports
        .read_port
        .ha_lamp_ids_for_short(body.adapter_id, short_address)
    {
        publish_lamp(client, session, topics, settings, ports, body, lamp_id);
    }
}

fn publish_lamp(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    body: &dali2rust_contracts::msg::RuntimeStateChangedEvent,
    lamp_id: u8,
) {
    let adapter_id = body.adapter_id;
    if !session.lamp_config_hashes.contains_key(&(adapter_id, lamp_id)) {
        republish_lamp_config(
            client, session, topics, settings, ports, adapter_id, lamp_id,
            RetractGate::IfAnnounced,
        );
    }
    if !session.lamp_config_hashes.contains_key(&(adapter_id, lamp_id)) {
        return;
    }
    let absent = body
        .state_observation
        .error
        .as_ref()
        .is_some_and(dali2rust_contracts::msg::CompactErrorPayload::is_device_absent);
    publish_lamp_availability_if_changed(
        client, session, ports, topics, settings, adapter_id, lamp_id, absent,
    );
    if !body.state_setpoint.states_a_value() {
        return;
    }
    let payload = light_state_json(&body.state_setpoint);
    publish(client, ports, &topics.lamp_state_topic(adapter_id, lamp_id), &payload, qos_of(settings), settings.retain_state);
}

fn config_hash(payload: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    payload.hash(&mut hasher);
    hasher.finish()
}

#[derive(Clone, Copy, PartialEq)]
enum RetractGate {
    IfAnnounced,
    Always,
}

const fn input_component(instance_type: u8) -> Option<&'static str> {
    match instance_type {
        1 => Some("event"),
        3 => Some("binary_sensor"),
        2 | 4 => Some("sensor"),
        _ => None,
    }
}

#[inline(never)]
fn build_input_discovery(
    ports: &MqttWorkerPorts,
    topics: &HaTopics,
    device: &HaDevice<'_>,
    settings: &HomeAssistantSettingsView,
    adapter_id: u8,
    short_address: u8,
    instance_number: u8,
) -> Option<(String, String)> {
    let view = ports.read_port.ha_input_view(adapter_id, short_address)?;
    let instance = view
        .instances
        .iter()
        .find(|i| i.instance_number == instance_number)?;
    let instance_type = instance.instance_type?;
    let component = input_component(instance_type)?;
    let topic = topics.discovery_topic(
        component,
        &HaTopics::input_object_id(adapter_id, short_address, instance_number),
    );
    if !(settings.expose_input_devices && view.ha_expose) {
        return Some((topic, DISCOVERY_RETRACTION.to_string()));
    }
    let (_, payload) = input_instance_discovery_json(
        topics,
        device,
        adapter_id,
        view.name.as_deref(),
        short_address,
        instance_number,
        instance_type,
    )?;
    Some((topic, payload))
}

#[inline(never)]
#[allow(clippy::too_many_arguments, reason = "the cache is threaded, not rebuilt per call")]
fn publish_input_event(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    body: &dali2rust_contracts::msg::DaliInputEventObservedEvent,
) {
    if !settings.expose_input_devices {
        return;
    }
    let (Some(short_address), Some(instance_number)) = (body.short_address, body.instance_number)
    else {
        return;
    };
    let adapter_id = body.registry_adapter_id;
    let Some(view) = ports.read_port.ha_input_view(adapter_id, short_address) else {
        return;
    };
    if !view.ha_expose {
        return;
    }
    let Some(payload) = input_event_state_payload(body.typed, body.typed_value) else {
        return;
    };
    let key = (adapter_id, short_address, instance_number);
    if !session.input_config_hashes.contains_key(&key) {
        republish_input_configs(client, session, topics, settings, ports, adapter_id, short_address);
    }
    if !session.input_config_hashes.contains_key(&key) {
        return;
    }
    let retain = settings.retain_state
        && body.typed != dali2rust_contracts::msg::InputEventKind::Button;
    let topic = topics.input_state_topic(adapter_id, short_address, instance_number);
    let _ = publish(client, ports, &topic, &payload, qos_of(settings), retain);
}

#[inline(never)]
fn republish_input_configs(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    adapter_id: u8,
    short_address: u8,
) {
    let device = ha_device(settings, ports);
    let instances: Vec<u8> = match ports.read_port.ha_input_view(adapter_id, short_address) {
        Some(view) => view.instances.iter().map(|i| i.instance_number).collect(),
        None => session
            .input_config_hashes
            .keys()
            .filter(|(a, s, _)| *a == adapter_id && *s == short_address)
            .map(|(_, _, i)| *i)
            .collect(),
    };
    for instance_number in instances {
        let key = (adapter_id, short_address, instance_number);
        match build_input_discovery(
            ports, topics, &device, settings, adapter_id, short_address, instance_number,
        ) {
            Some((topic, payload)) => {
                let hash = config_hash(&payload);
                if session.input_config_hashes.get(&key) == Some(&hash) {
                    continue;
                }
                if publish(client, ports, &topic, &payload, qos_of(settings), retain_config(&payload, settings)) {
                    if payload == DISCOVERY_RETRACTION {
                        session.input_config_hashes.remove(&key);
                    } else {
                        session.input_config_hashes.insert(key, hash);
                    }
                }
            }
            None => {
                session.input_config_hashes.remove(&key);
            }
        }
    }
}

#[allow(clippy::too_many_arguments, reason = "the cache is threaded, not rebuilt per call")]
fn publish_lamp_availability(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    ports: &MqttWorkerPorts,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    adapter_id: u8,
    lamp_id: u8,
    unreachable: bool,
) -> bool {
    let published = publish(
        client,
        ports,
        &topics.lamp_availability_topic(adapter_id, lamp_id),
        if unreachable { "offline" } else { "online" },
        qos_of(settings),
        true,
    );
    if published {
        session
            .lamp_availability
            .insert((adapter_id, lamp_id), unreachable);
    } else {
        session.lamp_availability.remove(&(adapter_id, lamp_id));
    }
    published
}

#[allow(clippy::too_many_arguments, reason = "mirrors the function it gates")]
fn publish_lamp_availability_if_changed(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    ports: &MqttWorkerPorts,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    adapter_id: u8,
    lamp_id: u8,
    unreachable: bool,
) {
    if session.lamp_availability.get(&(adapter_id, lamp_id)) == Some(&unreachable) {
        return;
    }
    publish_lamp_availability(
        client, session, ports, topics, settings, adapter_id, lamp_id, unreachable,
    );
}

fn clear_lamp_retained(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    ports: &MqttWorkerPorts,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    adapter_id: u8,
    lamp_id: u8,
) -> bool {
    session.lamp_availability.remove(&(adapter_id, lamp_id));
    let qos = qos_of(settings);
    let availability =
        publish(client, ports, &topics.lamp_availability_topic(adapter_id, lamp_id), "", qos, true);
    let state = publish(client, ports, &topics.lamp_state_topic(adapter_id, lamp_id), "", qos, true);
    availability && state
}

#[allow(clippy::too_many_arguments, reason = "the cache is threaded, not rebuilt per call")]
#[inline(never)]
fn republish_lamp_config(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    settings: &HomeAssistantSettingsView,
    ports: &MqttWorkerPorts,
    adapter_id: u8,
    lamp_id: u8,
    retract: RetractGate,
) {
    let object_id = HaTopics::lamp_object_id(adapter_id, lamp_id);
    let topic = topics.discovery_topic("light", &object_id);
    match ports.read_port.ha_lamp_view(adapter_id, lamp_id) {
        Some(lamp) => {
            let device = ha_device(settings, ports);
            let payload = lamp_discovery_json(topics, &device, adapter_id, &lamp);
            let hash = config_hash(&payload);
            if session.lamp_config_hashes.get(&(adapter_id, lamp_id)) == Some(&hash) {
                return;
            }
            let config_published =
                publish(client, ports, &topic, &payload, qos_of(settings), retain_config(&payload, settings));
            let paired = config_published
                && publish_lamp_availability(
                    client, session, ports, topics, settings, adapter_id, lamp_id, lamp.unreachable,
                );
            if paired {
                session.lamp_config_hashes.insert((adapter_id, lamp_id), hash);
            }
        }
        None => {
            let announced_here = session.lamp_config_hashes.remove(&(adapter_id, lamp_id)).is_some();
            if announced_here || retract == RetractGate::Always {
                publish(
                    client,
                    ports,
                    &topic,
                    DISCOVERY_RETRACTION,
                    qos_of(settings),
                    retain_config(DISCOVERY_RETRACTION, settings),
                );
                clear_lamp_retained(client, session, ports, topics, settings, adapter_id, lamp_id);
            }
        }
    }
}

pub(crate) struct RulePublishBudget {
    tokens: u32,
    last_refill_ms: u64,
}

impl Default for RulePublishBudget {
    fn default() -> Self {
        Self::new(0)
    }
}

const RULE_PUBLISH_BURST: u32 = 4;
const RULE_PUBLISH_REFILL_MS: u64 = 1_000;

impl RulePublishBudget {
    pub(crate) fn new(now_ms: u64) -> Self {
        Self {
            tokens: RULE_PUBLISH_BURST,
            last_refill_ms: now_ms,
        }
    }

    fn take(&mut self, now_ms: u64) -> bool {
        let refills = now_ms.saturating_sub(self.last_refill_ms) / RULE_PUBLISH_REFILL_MS;
        if refills > 0 {
            self.tokens = (self.tokens + u32::try_from(refills).unwrap_or(u32::MAX))
                .min(RULE_PUBLISH_BURST);
            self.last_refill_ms += refills * RULE_PUBLISH_REFILL_MS;
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

fn drain_rule_publishes(
    client: &mut Box<dyn MqttClient>,
    ports: &MqttWorkerPorts,
    budget: &mut RulePublishBudget,
    publishes: Vec<dali2rust_contracts::msg::MqttPublishCommand>,
) {
    for cmd in publishes {
        let now_ms = dali2rust_bsp::unix_clock::unix_wall_clock_millis();
        if !budget.take(now_ms) {
            bump(&ports.counters.rule_publishes_dropped_total);
            continue;
        }
        publish(
            client,
            ports,
            cmd.topic.as_str(),
            cmd.payload.as_str(),
            MqttQos::AtMostOnce,
            cmd.retain,
        );
    }
}

fn publish(
    client: &mut Box<dyn MqttClient>,
    ports: &MqttWorkerPorts,
    topic: &str,
    payload: &str,
    qos: MqttQos,
    retain: bool,
) -> bool {
    match client.publish(topic, payload.as_bytes(), qos, retain) {
        Ok(()) => {
            bump(&ports.counters.publishes_total);
            true
        }
        Err(e) => {
            log::warn!("mqtt: publish to {topic} refused: {e:?}");
            bump(&ports.counters.publish_failures_total);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{MockMqttClient, MockMqttHandle};
    use dali2rust_domain::registry::{
        HaGroupView, HaLampView, HaSceneView, HomeAssistantSettingsApplyWatchPort,
    };

    struct EmptyReadPort;

    impl HaPublishReadPort for EmptyReadPort {
        fn ha_input_views(
            &self,
            _adapter_id: u8,
        ) -> Vec<dali2rust_domain::registry::HaInputView> {
            Vec::new()
        }
        fn ha_input_view(
            &self,
            _adapter_id: u8,
            _short_address: u8,
        ) -> Option<dali2rust_domain::registry::HaInputView> {
            None
        }

        fn ha_lamp_views(&self, _adapter_id: u8) -> Vec<HaLampView> {
            Vec::new()
        }
        fn ha_group_views(&self, _adapter_id: u8) -> Vec<HaGroupView> {
            Vec::new()
        }
        fn ha_scene_views(&self, _adapter_id: u8) -> Vec<HaSceneView> {
            Vec::new()
        }
        fn ha_lamp_view(&self, _adapter_id: u8, _virtual_lamp_id: u8) -> Option<HaLampView> {
            None
        }
        fn ha_group_view(&self, _adapter_id: u8, _group_id: u8) -> Option<HaGroupView> {
            None
        }
        fn ha_lamp_ids_for_short(&self, _adapter_id: u8, _short_address: u8) -> Vec<u8> {
            Vec::new()
        }
        fn ha_retracted_lamp_ids(&self, _adapter_id: u8) -> Vec<u8> {
            Vec::new()
        }
        fn ha_retracted_group_ids(&self, _adapter_id: u8) -> Vec<u8> {
            Vec::new()
        }
        fn ha_active_scene(&self, _adapter_id: u8) -> Option<u8> {
            None
        }
    }

    struct FixedSettings;

    impl HomeAssistantSettingsReadPort for FixedSettings {
        fn home_assistant_settings_view(&self) -> HomeAssistantSettingsView {
            HomeAssistantSettingsView::default()
        }
    }

    struct StillWatch;

    impl HomeAssistantSettingsApplyWatchPort for StillWatch {
        fn home_assistant_settings_applied_load(&self) -> u32 {
            0
        }
    }

    struct NoSecret;

    impl HomeAssistantSecretReadPort for NoSecret {
        fn home_assistant_broker_password(&self) -> String {
            String::new()
        }
    }

    fn test_ports() -> (MqttWorkerPorts, dali2rust_bus::BusHost) {
        let (host, publisher, ()) =
            dali2rust_bus::BusHost::spawn(dali2rust_bus::BusConfig::default(), |_reg| ());
        (
            MqttWorkerPorts {
                publisher,
                bus_id: BusId::default(),
                correlation: Arc::new(dali2rust_bus::CorrelationIdAllocator::new()),
                read_port: Arc::new(EmptyReadPort),
                settings: Arc::new(FixedSettings),
                settings_watch: Arc::new(StillWatch),
                secret: Arc::new(NoSecret),
                counters: Arc::new(MqttCounters::default()),
                version: "test",
                adapter_count: 1,
                role: Arc::new(ActiveRole),
            },
            host,
        )
    }

    struct ActiveRole;

    impl dali2rust_domain::registry::DaliSettingsReadPort for ActiveRole {
        fn dali_settings_view(&self) -> dali2rust_domain::registry::DaliSettingsView {
            dali2rust_domain::registry::DaliSettingsView {
                dt8_auto_activation_repair: false,
                dt8_rgbwaf_control_assert: false,
                application_active: true,
                device_short_address: None,
            }
        }
    }

    struct PassiveRole;

    impl dali2rust_domain::registry::DaliSettingsReadPort for PassiveRole {
        fn dali_settings_view(&self) -> dali2rust_domain::registry::DaliSettingsView {
            dali2rust_domain::registry::DaliSettingsView {
                dt8_auto_activation_repair: false,
                dt8_rgbwaf_control_assert: false,
                application_active: false,
                device_short_address: None,
            }
        }
    }

    #[test]
    fn a_standby_holds_no_broker_session_however_configured_the_bridge_is() {
        let settings = HomeAssistantSettingsView {
            enabled: true,
            broker_host: "broker.example".to_string(),
            ..HomeAssistantSettingsView::default()
        };
        assert!(
            ready(&settings, &ActiveRole),
            "the fixture must be otherwise ready, or this proves nothing"
        );
        assert!(!ready(&settings, &PassiveRole));
    }

    #[test]
    fn connected_is_reported_only_after_the_broker_acks_the_command_subscriptions() {
        let (ports, _host) = test_ports();
        let (mock, _incoming) = MockMqttClient::new();
        mock.hold_subacks();
        let mut client: Box<dyn MqttClient> = Box::new(MockMqttHandle::new(Arc::clone(&mock)));
        let settings = HomeAssistantSettingsView {
            enabled: true,
            broker_host: "broker.example".to_string(),
            ..HomeAssistantSettingsView::default()
        };
        let mut session = Session::default();
        let mut topics = HaTopics::default();
        let mut job = None;
        let mut pacer = DialPacer::default();
        assert!(ensure_session(
            &mut client, &mut session, &mut topics, &mut job, &settings, "", &mut pacer, &ports
        ));
        assert!(!mock.subscriptions().is_empty(), "the fresh session subscribed");
        assert!(
            !ports.counters.is_connected(),
            "subscribed but not acked: the gauge must not read 1 yet"
        );
        mock.release_subacks();
        assert!(ensure_session(
            &mut client, &mut session, &mut topics, &mut job, &settings, "", &mut pacer, &ports
        ));
        assert!(ports.counters.is_connected(), "every SUBACK in: the gauge reads 1");
    }

    #[test]
    fn stand_down_clears_the_connected_gauge_even_when_the_link_already_dropped() {
        let (ports, _host) = test_ports();
        let (mock, _incoming) = MockMqttClient::new();
        let mut client: Box<dyn MqttClient> = Box::new(MockMqttHandle::new(Arc::clone(&mock)));
        let link = client.link();
        assert!(!link.is_connected(), "precondition: the drop was already reported");
        ports.counters.set_connected(true);
        let mut session = Session::default();
        let mut job = None;
        stand_down(
            &mut client,
            &mut session,
            &HaTopics::default(),
            &ports,
            &link,
            &mut job,
        );
        assert!(
            !ports.counters.is_connected(),
            "disabled with the link already down: the gauge must not stay 1"
        );
    }
}
