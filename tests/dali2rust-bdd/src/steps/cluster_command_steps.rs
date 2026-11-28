use std::cell::RefCell;
use std::time::{Duration, Instant};

use cucumber::{then, when};
use dali2rust_cluster_runtime::frame::{
    decode, encode, Action, BirthMark, Command, DatagramBuffer, Frame, Header, LampAddress, LightVerb, NoteDeed, NoteReach,
    RecallScope, RecordRef, SourceClass, Target, WireNote,
};
use dali2rust_contracts::msg::{ColorMode, ColorValue, LevelTransition, LightSetpoint, PowerState};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use dali2rust_test_support::{try_wait_until, HubDatagram};
use serde::Deserialize;
use serde_json::Value;

use crate::steps::cluster_steps::{hardware_address, rig, ClusterRig};
use crate::steps::polling::fetch_json;
use crate::DaliWorld;

const HUB_DEADLINE: Duration = Duration::from_secs(4);
const STATS_DEADLINE: Duration = Duration::from_secs(8);
const COPIES: usize = 2;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandSpec {
    channel: Option<[u32; 3]>,
    scene: Option<[u32; 3]>,
    lamp: Option<[u8; 3]>,
    rgb: Option<[u8; 3]>,
    kelvin: Option<u16>,
    level: Option<u8>,
    power: Option<String>,
    recall: Option<String>,
    dim: Option<String>,
    all: Option<bool>,
    source: Option<String>,
    hold_hcl: Option<bool>,
    executed_on_wire: Option<u8>,
}

fn record([number, generation, nonce]: [u32; 3]) -> RecordRef {
    let number = u8::try_from(number).expect("a record number");
    RecordRef { number, mark: BirthMark { generation, nonce } }
}

impl CommandSpec {
    fn parse(json: &str) -> Self {
        serde_json::from_str(json).expect("a command spec")
    }

    fn action(&self) -> Action {
        let channel = self.channel.map(record);
        match (self.scene.map(record), channel) {
            (Some(scene), None) => Action::RecallScene { scene, scope: RecallScope::Node },
            (Some(scene), Some(channel)) => Action::RecallScene { scene, scope: RecallScope::Channel(channel) },
            (None, Some(channel)) => Action::Light { target: Target::Channel(channel), verb: self.verb() },
            (None, None) if self.all == Some(true) => Action::Light { target: Target::All, verb: self.verb() },
            (None, None) if self.lamp.is_some() => Action::Light { target: self.lamp_target(), verb: self.verb() },
            (None, None) => panic!("a command names a channel, all lamps, a lamp or a scene"),
        }
    }

    fn lamp_target(&self) -> Target {
        let [node, adapter, lamp] = self.lamp.expect("a lamp address");
        Target::Lamp(LampAddress { node, adapter, lamp })
    }

    fn colour(&self) -> Option<ColorValue> {
        let rgb = self.rgb.map(|[r, g, b]| ColorValue { mode: ColorMode::Rgb, r, g, b, ..ColorValue::default() });
        let kelvin = self.kelvin.map(|kelvin| ColorValue {
            mode: ColorMode::Cct,
            color_temperature_kelvin: kelvin,
            ..ColorValue::default()
        });
        rgb.or(kelvin)
    }

    fn verb(&self) -> LightVerb {
        if let Some(dim) = self.dim.as_deref() {
            return relative_verb(dim);
        }
        if let Some(power) = self.power.as_deref() {
            let power = match power {
                "on" => PowerState::On,
                "off" => PowerState::Off,
                other => panic!("no power {other}"),
            };
            return LightVerb::Setpoint(LightSetpoint { power, level: self.level, color: self.colour() });
        }
        match (self.level, self.recall.as_deref()) {
            (Some(level), None) => {
                LightVerb::Setpoint(LightSetpoint { power: PowerState::On, level: Some(level), color: self.colour() })
            }
            (None, Some("max_level")) => LightVerb::Transition(LevelTransition::RecallMaxLevel),
            (None, Some("min_level")) => LightVerb::Transition(LevelTransition::RecallMinLevel),
            (None, Some("up")) => LightVerb::Transition(LevelTransition::Up),
            other => panic!("no verb for {other:?}"),
        }
    }

    fn source(&self) -> SourceClass {
        match self.source.as_deref() {
            None | Some("operator") => SourceClass::Operator,
            Some("home_assistant") => SourceClass::HomeAssistant,
            Some("hcl") => SourceClass::Hcl,
            Some("foreign_master") => SourceClass::ForeignMaster,
            Some(other) => panic!("no source class {other}"),
        }
    }

    fn command(&self) -> Command {
        let hold_hcl = self.hold_hcl.unwrap_or(true);
        Command { action: self.action(), source: self.source(), hold_hcl, executed_on_wire: self.executed_on_wire }
    }
}

fn relative_verb(dim: &str) -> LightVerb {
    match dim {
        "up" => LightVerb::Transition(LevelTransition::Up),
        "down" => LightVerb::Transition(LevelTransition::Down),
        "step_up" => LightVerb::Transition(LevelTransition::StepUp),
        "step_down" => LightVerb::Transition(LevelTransition::StepDown),
        "on_and_step_up" => LightVerb::Transition(LevelTransition::OnAndStepUp),
        "step_down_and_off" => LightVerb::Transition(LevelTransition::StepDownAndOff),
        "stop_fade" => LightVerb::StopFade,
        other => panic!("no relative verb {other}"),
    }
}

fn command_datagram(header: &Header, command: &Command) -> Vec<u8> {
    frame_datagram(header, &Frame::Command(command.clone()))
}

fn frame_datagram(header: &Header, frame: &Frame<'_>) -> Vec<u8> {
    let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
    let len = encode(header, frame, &mut buffer).expect("the frame fits");
    buffer.get(..len).expect("the encoded length").to_vec()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteSpec {
    wire: u8,
    group: u8,
    dim: String,
}

impl NoteSpec {
    fn note(&self) -> WireNote {
        let transition = match self.dim.as_str() {
            "up" => LevelTransition::Up,
            "down" => LevelTransition::Down,
            other => panic!("unknown dimming {other}"),
        };
        let deed = NoteDeed::Light(LightVerb::Transition(transition));
        WireNote { wire: self.wire, adapter: 0, reach: NoteReach::Group(self.group), deed }
    }
}

// CLU-180
#[when(regex = r#"^the board "([0-9a-f:]{17})" notes the frame (\{.*\}) twice with one sequence number$"#)]
async fn board_notes_frame_twice(world: &mut DaliWorld, mac: String, json: String) {
    let spec: NoteSpec = serde_json::from_str(&json).expect("a note spec");
    let note = Frame::WireNote(spec.note());
    let speaker = rig(world).speaker(hardware_address(&mac));
    let mut bytes = Vec::new();
    speaker.speak(|header| {
        bytes = frame_datagram(header, &note);
        bytes.clone()
    });
    speaker.repeat(&bytes);
}

// CLU-076 CLU-077 CLU-078 CLU-079 CLU-080 CLU-081 CLU-082 CLU-086 CLU-087 CLU-088 CLU-089 CLU-098 CLU-100 CLU-102 CLU-103 CLU-109 CLU-111 CLU-178 CLU-179 CLU-187 CLU-220 CLU-221 CLU-222 CLU-223 CLU-224 CLU-225 CLU-226
#[when(regex = r#"^the board "([0-9a-f:]{17})" sends the command (\{.*\}) twice with one sequence number$"#)]
async fn board_sends_command_twice(world: &mut DaliWorld, mac: String, json: String) {
    let command = CommandSpec::parse(&json).command();
    let speaker = rig(world).speaker(hardware_address(&mac));
    let mut bytes = Vec::new();
    speaker.speak(|header| {
        bytes = command_datagram(header, &command);
        bytes.clone()
    });
    speaker.repeat(&bytes);
}

// CLU-111
#[when(regex = r#"^the board "([0-9a-f:]{17})" loses a command on the way$"#)]
async fn board_loses_command(world: &mut DaliWorld, mac: String) {
    rig(world).speaker(hardware_address(&mac)).lose();
}

// CLU-078 CLU-223
#[when(regex = r#"^a datagram with this controller's own hardware address carries the command (\{.*\}) as node (\d) of cluster "([^"]*)"$"#)]
async fn own_address_sends_command(world: &mut DaliWorld, json: String, node: u8, name: String) {
    let command = CommandSpec::parse(&json).command();
    let speaker = rig(world).impostor(node, name);
    speaker.speak(|header| command_datagram(header, &command));
}

fn sent_commands(rig: &ClusterRig, window: Duration, enough: usize) -> Vec<(u32, Command, HubDatagram)> {
    let deadline = Instant::now() + window;
    let mut seen = Vec::new();
    while seen.len() < enough {
        let Some(sent) = rig.sent_by_us(deadline.saturating_duration_since(Instant::now())) else {
            break;
        };
        let Ok(decoded) = decode(&sent.bytes) else {
            continue;
        };
        if let Frame::Command(command) = decoded.frame {
            seen.push((decoded.header.seq, colour_stated_or_none(command), sent.clone()));
        }
    }
    seen
}

fn colour_stated_or_none(mut command: Command) -> Command {
    if let Action::Light { verb: LightVerb::Setpoint(setpoint), .. } = &mut command.action {
        if !setpoint.states_color() {
            setpoint.color = None;
        }
    }
    command
}

// CLU-083 CLU-090 CLU-092 CLU-094 CLU-095 CLU-097 CLU-099 CLU-101 CLU-170 CLU-171 CLU-172 CLU-173 CLU-176 CLU-177 CLU-178 CLU-179 CLU-185 CLU-227 CLU-228
#[then(regex = r"^the cluster hub should carry from this controller the command (\{.*\}) twice with one sequence number$")]
async fn hub_carries_command_twice(world: &mut DaliWorld, json: String) {
    let wanted = CommandSpec::parse(&json).command();
    let sent = sent_commands(rig(world), HUB_DEADLINE, 2);
    let [(first_seq, first, first_bytes), (second_seq, second, second_bytes)] = sent.as_slice() else {
        panic!("{} command datagrams from this controller within {HUB_DEADLINE:?}", sent.len());
    };
    assert_eq!(first, &wanted, "the command on the hub");
    assert_eq!(second, &wanted, "the second copy");
    assert_eq!(first_seq, second_seq, "the copies share one sequence number");
    assert_eq!(first_bytes.bytes, second_bytes.bytes, "the copies are the same datagram");
}

// CLU-076 CLU-081 CLU-091 CLU-093 CLU-094 CLU-095 CLU-096 CLU-097 CLU-098 CLU-099 CLU-100 CLU-102 CLU-108 CLU-109 CLU-110 CLU-171 CLU-173 CLU-174 CLU-175 CLU-177 CLU-178 CLU-179 CLU-185 CLU-187 CLU-180 CLU-220 CLU-226 CLU-228
#[then(regex = r"^the cluster hub should carry no command from this controller within (\d+) ms$")]
async fn hub_carries_no_command(world: &mut DaliWorld, window_ms: u64) {
    let sent = sent_commands(rig(world), Duration::from_millis(window_ms), 1);
    assert!(sent.is_empty(), "this controller sent a command: {:?}", sent.first().map(|(_, command, _)| command));
}

fn command_pairs(sent: &[(u32, Command, HubDatagram)]) -> Vec<(u32, Command)> {
    sent.chunks(COPIES)
        .map(|pair| {
            let [(seq, command, bytes), (copy_seq, copy, copy_bytes)] = pair else {
                panic!("a command without its copy: {pair:?}");
            };
            assert_eq!((seq, command), (copy_seq, copy), "the copy follows its command");
            assert_eq!(bytes.bytes, copy_bytes.bytes, "the copies are the same datagram");
            (*seq, command.clone())
        })
        .collect()
}

fn assert_rising(pairs: &[(u32, Command)]) {
    let seqs: Vec<u32> = pairs.iter().map(|(seq, _)| *seq).collect();
    assert!(seqs.windows(2).all(|two| two.first() < two.get(1)), "the commands left in their order: {seqs:?}");
}

// CLU-104 CLU-105 CLU-230
#[then(regex = r"^the cluster hub should carry from this controller the commands (\[.*\]) in this order, each twice with one sequence number$")]
async fn hub_carries_commands_in_order(world: &mut DaliWorld, json: String) {
    let specs: Vec<serde_json::Value> = serde_json::from_str(&json).expect("a list of command specs");
    let wanted: Vec<Command> = specs.iter().map(|spec| CommandSpec::parse(&spec.to_string()).command()).collect();
    let sent = sent_commands(rig(world), HUB_DEADLINE, wanted.len() * COPIES);
    let pairs = command_pairs(&sent);
    assert_rising(&pairs);
    let commands: Vec<Command> = pairs.into_iter().map(|(_, command)| command).collect();
    assert_eq!(commands, wanted, "the commands on the hub");
}

fn cluster_counter(json: &Value, name: &str) -> u64 {
    json.pointer(&format!("/cluster/{name}")).and_then(Value::as_u64).unwrap_or(u64::MAX)
}

fn settled_sends(world: &DaliWorld, forwarded: u64) -> Value {
    let port = world.server_port();
    let last = RefCell::new(Value::Null);
    let settled = try_wait_until(
        || {
            let stats = fetch_json(port, "/api/v1/stats").unwrap_or(Value::Null);
            let sent = cluster_counter(&stats, "commands_sent_total");
            let dropped = cluster_counter(&stats, "relative_verbs_dropped_total");
            let done = cluster_counter(&stats, "commands_forwarded_total") == forwarded
                && sent.saturating_add(dropped) == forwarded;
            *last.borrow_mut() = stats;
            done
        },
        STATS_DEADLINE,
    );
    let stats = last.into_inner();
    assert!(settled, "{forwarded} forwarded, each sent or dropped: {}", stats["cluster"]);
    stats
}

// CLU-106
#[then(regex = r"^the cluster hub should carry from this controller (\d+) forwarded commands (\{.*\}) as sent or dropped, in their order, each twice with one sequence number$")]
async fn hub_carries_sent_or_dropped(world: &mut DaliWorld, forwarded: u64, json: String) {
    let wanted = CommandSpec::parse(&json).command();
    let stats = settled_sends(world, forwarded);
    assert_eq!(cluster_counter(&stats, "commands_superseded_total"), 0, "no relative verb was superseded");
    let sent = usize::try_from(cluster_counter(&stats, "commands_sent_total")).expect("a count");
    let pairs = command_pairs(&sent_commands(rig(world), HUB_DEADLINE, sent * COPIES + 1));
    assert_eq!(pairs.len(), sent, "every command counted sent is on the hub, and no more");
    assert_rising(&pairs);
    assert!(pairs.iter().all(|(_, command)| command == &wanted), "only {wanted:?} left: {pairs:?}");
}
