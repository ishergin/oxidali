use std::time::{Duration, Instant};

use cucumber::{then, when};
use dali2rust_cluster_runtime::frame::{
    decode, encode, Action, BirthMark, Command, DatagramBuffer, Frame, Header, LightVerb, RecallScope, RecordRef,
    SourceClass, Target,
};
use dali2rust_contracts::msg::{LevelTransition, LightSetpoint, PowerState};
use dali2rust_platform::cluster_link::MAX_CLUSTER_DATAGRAM_BYTES;
use dali2rust_test_support::HubDatagram;
use serde::Deserialize;

use crate::steps::cluster_steps::{hardware_address, rig, ClusterRig};
use crate::DaliWorld;

const HUB_DEADLINE: Duration = Duration::from_secs(4);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandSpec {
    channel: Option<[u32; 3]>,
    scene: Option<[u32; 3]>,
    level: Option<u8>,
    recall: Option<String>,
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
            (None, None) => panic!("a command names a channel, all lamps or a scene"),
        }
    }

    fn verb(&self) -> LightVerb {
        match (self.level, self.recall.as_deref()) {
            (Some(level), None) => LightVerb::Setpoint(LightSetpoint { power: PowerState::On, level: Some(level), color: None }),
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
            Some(other) => panic!("no source class {other}"),
        }
    }

    fn command(&self) -> Command {
        let hold_hcl = self.hold_hcl.unwrap_or(true);
        Command { action: self.action(), source: self.source(), hold_hcl, executed_on_wire: self.executed_on_wire }
    }
}

fn command_datagram(header: &Header, command: &Command) -> Vec<u8> {
    let mut buffer: Box<DatagramBuffer> = Box::new([0; MAX_CLUSTER_DATAGRAM_BYTES]);
    let len = encode(header, &Frame::Command(command.clone()), &mut buffer).expect("the command fits");
    buffer.get(..len).expect("the encoded length").to_vec()
}

// CLU-076 CLU-077 CLU-078 CLU-079 CLU-080 CLU-081 CLU-082 CLU-086 CLU-087 CLU-088 CLU-089 CLU-098 CLU-100
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

// CLU-078
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

// CLU-083 CLU-090 CLU-092 CLU-094 CLU-095 CLU-097 CLU-099
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

// CLU-076 CLU-081 CLU-091 CLU-093 CLU-094 CLU-095 CLU-096 CLU-097 CLU-098 CLU-099 CLU-100
#[then(regex = r"^the cluster hub should carry no command from this controller within (\d+) ms$")]
async fn hub_carries_no_command(world: &mut DaliWorld, window_ms: u64) {
    let sent = sent_commands(rig(world), Duration::from_millis(window_ms), 1);
    assert!(sent.is_empty(), "this controller sent a command: {:?}", sent.first().map(|(_, command, _)| command));
}
