use std::sync::Arc;
use std::time::{Duration, Instant};

use dali2rust_api::ha::HaTopics;
use dali2rust_bus::worker_counters::bump;
use dali2rust_bus::BusSubscriberRx;
use dali2rust_domain::registry::HomeAssistantSettingsView;
use dali2rust_platform::mqtt::{MqttClient, MqttConnectionState, MqttLink};

use crate::runtime::bus_out::refuse_command;
use crate::runtime::discovery::{plan_discovery, DiscoveryJob};
use crate::runtime::intake::discard_bus;
use crate::runtime::rule_topics::RuleTopicCache;
use crate::runtime::session::{
    announce_and_subscribe, announce_offline, session_config, session_settings_fingerprint,
    topics_of, Session,
};
use crate::runtime::worker::{MqttWorkerPorts, WORKER_STACK};

pub const PARK: Duration = Duration::from_millis(500);

const REDIAL_BACKOFF: &[Duration] = &[Duration::from_secs(2), Duration::from_secs(8)];

const STALLED_JOB_DIAL_STREAK: u32 = 2;

#[derive(Default)]
pub(super) struct DialPacer {
    last_attempt: Option<Instant>,
    failures: u32,
    dialled: u64,
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

pub(super) fn ready(
    settings: &HomeAssistantSettingsView,
    role: &dyn dali2rust_domain::registry::DaliSettingsReadPort,
) -> bool {
    settings.enabled
        && !settings.broker_host.is_empty()
        && role.dali_settings_view().application_active
}

#[inline(never)]
pub(super) fn restart_if_settings_changed(
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
pub(super) fn ensure_session(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &mut HaTopics,
    job: &mut Option<DiscoveryJob>,
    settings: &HomeAssistantSettingsView,
    password: &str,
    pacer: &mut DialPacer,
    ports: &MqttWorkerPorts,
    rules: &mut RuleTopicCache,
) -> bool {
    let link = client.link();
    if !link.is_connected() || link.session_generation() != session.generation {
        end_rule_session(rules, ports);
    }
    if !link.is_connected() {
        ports.counters.set_connected(false);
        if !dial(client, &link, settings, password, pacer, ports) {
            return false;
        }
    }
    pacer.established();
    if link.session_generation() != session.generation
        && !begin_session(client, &link, session, topics, job, settings, password, ports)
    {
        return false;
    }
    follow_rule_topics(client.as_mut(), session, rules, ports);
    settle_subacks(&link, session, ports);
    ports.counters.set_connected(
        session.subscriptions.all_granted() && session.rule_topics.complete(rules.topics()),
    );
    true
}

#[allow(clippy::too_many_arguments, reason = "the loop's state is threaded, not rebuilt")]
#[inline(never)]
fn begin_session(
    client: &mut Box<dyn MqttClient>,
    link: &MqttLink,
    session: &mut Session,
    topics: &mut HaTopics,
    job: &mut Option<DiscoveryJob>,
    settings: &HomeAssistantSettingsView,
    password: &str,
    ports: &MqttWorkerPorts,
) -> bool {
    let announced = topics_of(settings);
    let sent = match announce_and_subscribe(client.as_mut(), &announced) {
        Ok(sent) => sent,
        Err(e) => {
            log::warn!("mqtt: could not announce on a fresh session: {e:?}");
            announce_offline(client.as_mut(), &announced);
            return false;
        }
    };
    session.begin(
        link.session_generation(),
        session_settings_fingerprint(settings, password),
        &announced,
    );
    for (filter, message_id) in &sent {
        session.subscriptions.sent(filter, *message_id);
    }
    *topics = announced;
    plan_session_reannounce(job, ports);
    true
}

#[inline(never)]
fn plan_session_reannounce(job: &mut Option<DiscoveryJob>, ports: &MqttWorkerPorts) {
    let carried = job.take().and_then(|interrupted| interrupted.workflow);
    *job = Some(plan_discovery(ports, ports.adapter_count, carried));
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

#[inline(never)]
fn follow_rule_topics(
    client: &mut dyn MqttClient,
    session: &mut Session,
    rules: &mut RuleTopicCache,
    ports: &MqttWorkerPorts,
) {
    let lost = rules.refresh(ports.rule_topics.as_ref());
    dali2rust_bus::worker_counters::bump_by(&ports.counters.rule_messages_lost_total, lost);
    let refused =
        session.rule_topics.follow(client, &mut session.subscriptions, rules.topics(), &session.filters);
    dali2rust_bus::worker_counters::bump_by(&ports.counters.own_topics_refused_total, refused);
    ports.counters.set_own_topics_refused(session.rule_topics.own());
}

fn end_rule_session(rules: &mut RuleTopicCache, ports: &MqttWorkerPorts) {
    let lost = rules.pacer.clear();
    dali2rust_bus::worker_counters::bump_by(&ports.counters.rule_messages_lost_total, lost);
    ports.counters.set_own_topics_refused(&[]);
}

#[inline(never)]
fn settle_subacks(link: &MqttLink, session: &mut Session, ports: &MqttWorkerPorts) {
    for ack in link.take_subacks() {
        if let Some(filter) = session.subscriptions.settle(ack) {
            log::warn!("mqtt: the broker refused the subscription to {filter}");
            bump(&ports.counters.subscriptions_refused_total);
        }
    }
}

#[inline(never)]
pub(super) fn park_after_failed_dial(
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
pub(super) fn stand_down(
    client: &mut Box<dyn MqttClient>,
    session: &mut Session,
    topics: &HaTopics,
    ports: &MqttWorkerPorts,
    link: &Arc<MqttLink>,
    job: &mut Option<DiscoveryJob>,
    rules: &mut RuleTopicCache,
) {
    end_rule_session(rules, ports);
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
    } else if link.state() == MqttConnectionState::Disconnected {
        client.disconnect();
    }
    link.wait_for_change(PARK);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{MockMqttClient, MockMqttHandle};
    use crate::counters::MqttCounters;
    use crate::runtime::rule_traffic::pace_rule_message;
    use dali2rust_bus::{BusId, BusPublisher};
    use dali2rust_domain::registry::{
        HaGroupView, HaLampView, HaPublishReadPort, HaSceneView, HomeAssistantSecretReadPort,
        HomeAssistantSettingsApplyWatchPort, HomeAssistantSettingsReadPort,
    };
    use dali2rust_platform::liveness::monotonic_ms;
    use dali2rust_platform::mqtt::MqttIncoming;

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
        fn ha_retracted_input_views(
            &self,
            _adapter_id: u8,
        ) -> Vec<dali2rust_domain::registry::HaInputView> {
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
        (ports_on(publisher), host)
    }

    fn ports_on(publisher: BusPublisher) -> MqttWorkerPorts {
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
            rule_topics: Arc::new(crate::client::MockRuleTopics::default()),
        }
    }

    struct ActiveRole;

    impl dali2rust_domain::registry::DaliSettingsReadPort for ActiveRole {
        fn dali_settings_view(&self) -> dali2rust_domain::registry::DaliSettingsView {
            dali2rust_domain::registry::DaliSettingsView {
                dt8_auto_activation_repair: false,
                dt8_rgbwaf_control_assert: false,
                application_active: true,
                device_short_address: None,
                power_cycle_notification: false,
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
                power_cycle_notification: false,
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
        let mut rules = RuleTopicCache::default();
        assert!(ensure_session(
            &mut client, &mut session, &mut topics, &mut job, &settings, "", &mut pacer, &ports,
            &mut rules,
        ));
        assert!(!mock.subscriptions().is_empty(), "the fresh session subscribed");
        assert!(
            !ports.counters.is_connected(),
            "subscribed but not acked: the gauge must not read 1 yet"
        );
        mock.release_subacks();
        assert!(ensure_session(
            &mut client, &mut session, &mut topics, &mut job, &settings, "", &mut pacer, &ports,
            &mut rules,
        ));
        assert!(ports.counters.is_connected(), "every SUBACK in: the gauge reads 1");
    }

    const RULE_MESSAGE_WAIT_PROOF_MS: u32 = 1_000;

    struct Following {
        ports: MqttWorkerPorts,
        _host: dali2rust_bus::BusHost,
        rules: RuleTopicCache,
        published: BusSubscriberRx,
    }

    fn ports_following(topic: &str) -> Following {
        let (host, publisher, published) =
            dali2rust_bus::BusHost::spawn(dali2rust_bus::BusConfig::default(), |reg| {
                reg.subscribe_events(8, &["MqttRuleMessageEvent"])
            });
        let mut ports = ports_on(publisher);
        let followed = Arc::new(crate::client::MockRuleTopics::default());
        followed.set(&[topic]);
        ports.rule_topics = followed;
        let mut rules = RuleTopicCache::default();
        rules.refresh(ports.rule_topics.as_ref());
        Following { ports, _host: host, rules, published }
    }

    fn pace_two_rule_messages(f: &mut Following, topic: &str) {
        let now = monotonic_ms();
        for payload in ["first", "held"] {
            let message = MqttIncoming {
                topic: topic.to_string(),
                payload: payload.as_bytes().to_vec(),
                retained: false,
                session: 0,
            };
            pace_rule_message(&mut f.rules, &f.ports, message, now);
        }
    }

    fn assert_one_lost_and_the_count_adds_up(f: &mut Following) {
        let published = f.published.recv_timeout(Duration::from_secs(5)).is_ok();
        assert!(published, "the first message went out at once");
        let load = |counter: &std::sync::atomic::AtomicU32| MqttCounters::load(counter);
        let c = &f.ports.counters;
        let (total, coalesced, lost) =
            (load(&c.rule_messages_total), load(&c.rule_messages_coalesced_total), load(&c.rule_messages_lost_total));
        assert_eq!((total, coalesced, lost), (2, 0, 1));
        assert_eq!(total, 1 + coalesced + lost, "total = published + coalesced + lost");
        let later = monotonic_ms().wrapping_add(RULE_MESSAGE_WAIT_PROOF_MS);
        assert!(f.rules.pacer.take_due(later).is_empty(), "the held message never goes out");
    }

    #[test]
    fn a_message_held_when_the_session_ends_is_lost_and_never_reaches_the_next_session() {
        let mut f = ports_following("home/scene");
        pace_two_rule_messages(&mut f, "home/scene");
        let (mock, _incoming) = MockMqttClient::new();
        let mut client: Box<dyn MqttClient> = Box::new(MockMqttHandle::new(Arc::clone(&mock)));
        let settings = HomeAssistantSettingsView {
            enabled: true,
            broker_host: "broker.example".to_string(),
            ..HomeAssistantSettingsView::default()
        };
        let (mut session, mut topics, mut job) = (Session::default(), HaTopics::default(), None);
        let mut pacer = DialPacer::default();
        assert!(ensure_session(
            &mut client, &mut session, &mut topics, &mut job, &settings, "", &mut pacer, &f.ports,
            &mut f.rules,
        ));
        assert_one_lost_and_the_count_adds_up(&mut f);
    }

    #[test]
    fn standing_down_loses_a_held_rule_message_with_a_count() {
        let mut f = ports_following("home/scene");
        pace_two_rule_messages(&mut f, "home/scene");
        let (mock, _incoming) = MockMqttClient::new();
        let mut client: Box<dyn MqttClient> = Box::new(MockMqttHandle::new(Arc::clone(&mock)));
        let link = client.link();
        let (mut session, mut job) = (Session::default(), None);
        stand_down(&mut client, &mut session, &HaTopics::default(), &f.ports, &link, &mut job, &mut f.rules);
        assert_one_lost_and_the_count_adds_up(&mut f);
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
            &mut RuleTopicCache::default(),
        );
        assert!(
            !ports.counters.is_connected(),
            "disabled with the link already down: the gauge must not stay 1"
        );
    }
}
