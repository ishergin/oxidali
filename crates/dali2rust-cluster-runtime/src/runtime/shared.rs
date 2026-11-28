use core::hash::{Hash, Hasher};
use core::mem::MaybeUninit;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::collections::hash_map::DefaultHasher;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use dali2rust_bsp::psram::PsramBox;
use dali2rust_contracts::msg::{FixedText32, FixedText64};

use crate::frame::{decode, Command, Decoded, Frame, Header, InputEvent, InputInfoPage, WireNote, NODE_COUNT};
use crate::info::{Arrival, ClusterInfo};
use crate::inputs::{ClusterInputs, InputArrival, InputHeard};
use crate::lamps::{ClusterLamps, LampArrival};
use crate::presence::{
    claim_mode, elapsed_ms, in_reach, Arriving, BoardTable, HeardBoard, IgnoredReason, OwnFacts, OwnFaults, OwnMode,
    PresenceTiming, Roster, Taken, WireOwner, MAX_HEARD_BOARDS,
};

use super::composition::EpochSource;
use super::counters::{bump, ClusterCounters};
use super::forwarding::ForeignStanding;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum BlockState {
    #[default]
    Off,
    Passive,
    Listening,
    Contested,
    Settled,
}

impl BlockState {
    #[must_use]
    pub const fn issues_numbers(self) -> bool {
        matches!(self, Self::Off | Self::Passive | Self::Settled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OwnStatus {
    pub mode: OwnMode,
    pub node: u8,
    pub wire: u8,
    pub mac: Option<[u8; 6]>,
    pub boot_epoch: Option<u32>,
    pub wire_owner: WireOwner,
    pub faults: OwnFaults,
    pub held_by: Option<[u8; 6]>,
    pub block: BlockState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeView {
    pub node: u8,
    pub mac: [u8; 6],
    pub source: [u8; 4],
    pub installation: FixedText32,
    pub wire: u8,
    pub wire_protection: bool,
    pub ha_face: Option<bool>,
    pub live: bool,
    pub age_ms: u32,
    pub sequence_gaps: u32,
    pub split: bool,
    pub settled_for_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredView {
    pub mac: [u8; 6],
    pub source: [u8; 4],
    pub node: u8,
    pub cluster_name: FixedText64,
    pub installation: FixedText32,
    pub reason: IgnoredReason,
    pub age_ms: u32,
}

pub trait ClusterPresenceReadPort: Send + Sync {
    fn own_status(&self) -> OwnStatus;

    fn heard_node(&self, node: u8) -> Option<NodeView>;

    fn ignored_board(&self, index: usize) -> Option<IgnoredView>;

    fn neighbour_names_wire(&self, node: u8, wire: u8) -> bool;
}

pub struct PresenceState {
    pub(crate) table: BoardTable,
    pub(crate) own: OwnFacts,
    pub(crate) mode: OwnMode,
    pub(crate) mac_known: bool,
    pub(crate) link: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct Caches<'a> {
    pub(crate) info: &'a ClusterInfo,
    pub(crate) lamps: &'a ClusterLamps,
    pub(crate) inputs: &'a ClusterInputs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Eras {
    info: u32,
    lamps: u32,
    inputs: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Delivery {
    pub(crate) source: [u8; 4],
    pub(crate) link: u32,
    pub(crate) now_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Settled {
    pub(crate) mode: OwnMode,
    pub(crate) entered_running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Forgotten {
    Removed,
    StillHeard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandStanding {
    pub(crate) running: bool,
    pub(crate) owns_wire: bool,
    pub(crate) wire: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandAdmission {
    NotMember,
    Duplicate,
    Taken(CommandStanding),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Heard {
    Command(Command),
    Note(WireNote),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HeardCommand {
    pub(crate) heard: Heard,
    pub(crate) admission: CommandAdmission,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeardInput {
    pub(crate) node: u8,
    pub(crate) event: InputEvent,
    pub(crate) now_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Arrived {
    Command(HeardCommand),
    Input(HeardInput),
}

struct Admitted {
    taken: Option<Taken>,
    member: bool,
    standing: Option<CommandStanding>,
}

impl Admitted {
    const NOTHING: Self = Self { taken: None, member: false, standing: None };

    fn command(&self, heard: Heard) -> Option<HeardCommand> {
        let taken = self.taken?;
        let admission = if !self.member {
            CommandAdmission::NotMember
        } else if !taken.admits_command() {
            CommandAdmission::Duplicate
        } else {
            self.standing.map_or(CommandAdmission::NotMember, CommandAdmission::Taken)
        };
        Some(HeardCommand { heard, admission })
    }

    fn feeds_info(&self) -> bool {
        self.member && self.taken.is_some_and(Taken::applies)
    }
}

pub struct ClusterPresence {
    state: Mutex<PsramBox<PresenceState>>,
    timing: PresenceTiming,
    epoch_source: EpochSource,
    boot_epoch: OnceLock<u32>,
    hello_heard: AtomicBool,
    block_settled: AtomicBool,
}

impl ClusterPresence {
    pub fn new(timing: PresenceTiming, epoch_source: EpochSource) -> Self {
        Self {
            state: Mutex::new(placed_state()),
            timing,
            epoch_source,
            boot_epoch: OnceLock::new(),
            hello_heard: AtomicBool::new(false),
            block_settled: AtomicBool::new(false),
        }
    }

    pub const fn timing(&self) -> PresenceTiming {
        self.timing
    }

    pub(crate) fn draw_boot_epoch(&self) -> u32 {
        *self.boot_epoch.get_or_init(|| (self.epoch_source)())
    }

    fn drawn_boot_epoch(&self) -> Option<u32> {
        self.boot_epoch.get().copied()
    }

    fn lock(&self) -> MutexGuard<'_, PsramBox<PresenceState>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn link(&self) -> u32 {
        self.lock().link
    }

    pub(crate) fn take_datagram(
        &self,
        bytes: &[u8],
        delivery: Delivery,
        counters: &ClusterCounters,
        caches: Caches<'_>,
    ) -> Option<Arrived> {
        let decoded = match decode(bytes) {
            Ok(decoded) => decoded,
            Err(rejection) => {
                counters.count_rejection(rejection);
                return None;
            }
        };
        let (admitted, at) = self.admit(&decoded, delivery, counters, caches)?;
        if let Some(taken) = admitted.taken {
            counters.count_taken(taken);
            note_run(&decoded, taken, delivery.now_ms, caches);
        }
        match decoded.frame {
            Frame::Command(command) => return admitted.command(Heard::Command(command)).map(Arrived::Command),
            Frame::WireNote(note) => return admitted.command(Heard::Note(note)).map(Arrived::Command),
            Frame::InputEvent(event) => return hear_input(&decoded.header, event, &admitted, at, (counters, caches.inputs)),
            _ => {}
        }
        if admitted.feeds_info() {
            self.hear_member(&decoded, at, counters, caches);
        } else if matches!(decoded.frame, Frame::ChannelSceneInfo(_)) {
            counters.count_unfed_page(admitted.member, admitted.taken);
        }
        None
    }

    fn admit(
        &self,
        decoded: &Decoded<'_>,
        delivery: Delivery,
        counters: &ClusterCounters,
        caches: Caches<'_>,
    ) -> Option<(Admitted, HeardAt)> {
        let mut state = self.lock();
        if state.link != delivery.link {
            drop(state);
            bump(&counters.stale_link_frames_total);
            return None;
        }
        let admitted = state.take(decoded, delivery, self.timing, counters);
        let eras = Eras { info: caches.info.era(), lamps: caches.lamps.era(), inputs: caches.inputs.era(decoded.header.node_no) };
        Some((admitted, HeardAt { now_ms: delivery.now_ms, eras, own_node: state.own.node_no }))
    }

    fn hear_member(&self, decoded: &Decoded<'_>, at: HeardAt, counters: &ClusterCounters, caches: Caches<'_>) {
        let header = &decoded.header;
        let lamp_at = LampArrival { node: header.node_no, mac: header.sender_mac, epoch: header.boot_epoch, now_ms: at.now_ms };
        match &decoded.frame {
            Frame::Hello(_) => self.hello_heard.store(true, Relaxed),
            Frame::ChannelSceneInfo(page) => feed_info(caches.info, header, page, at, counters),
            Frame::LampState(page) => drop(caches.lamps.take_state(lamp_at, page, at.eras.lamps)),
            Frame::LampInfo(page) => drop(caches.lamps.take_info(lamp_at, page, at.eras.lamps)),
            Frame::InputInfo(page) => feed_inputs(caches.inputs, header, page, at),
            _ => {}
        }
    }

    pub(crate) fn note_block_settled(&self, settled: bool) {
        self.block_settled.store(settled, Relaxed);
    }

    fn block_settled(&self) -> bool {
        self.block_settled.load(Relaxed)
    }

    pub(crate) fn take_hello_heard(&self) -> bool {
        self.hello_heard.swap(false, Relaxed)
    }

    pub(crate) fn foreign_standing(&self, now_ms: u32) -> ForeignStanding {
        let state = self.lock();
        let roster = state.roster(now_ms, self.timing);
        ForeignStanding { forwards: roster.forwards_foreign(), shared: roster.shares_wire() }
    }

    pub(crate) fn member_heard(&self, node: u8, now_ms: u32) -> bool {
        let state = self.lock();
        let roster = state.roster(now_ms, self.timing);
        roster.displayed(node).is_some_and(|board| roster.live(board))
    }

    pub(crate) fn settle(&self, identity: OwnFacts, base: Option<OwnMode>, listened_ms: u32, now_ms: u32) -> Settled {
        let mut state = self.lock();
        let running_since_ms = state.own.running_since_ms;
        state.mac_known = identity.mac != [0; 6];
        state.own = OwnFacts { running_since_ms, ..identity };
        let mode = base.unwrap_or_else(|| claim_mode(&state.roster(now_ms, self.timing), state.mode, listened_ms));
        let entered_running = mode == OwnMode::Running && state.mode != OwnMode::Running;
        state.own.running_since_ms = match mode {
            OwnMode::Running if entered_running => Some(now_ms),
            OwnMode::Running => running_since_ms,
            _ => None,
        };
        state.mode = mode;
        Settled { mode, entered_running }
    }

    pub(crate) fn clear(&self) {
        let mut state = self.lock();
        state.table.clear();
        state.link = state.link.wrapping_add(1);
    }

    pub(crate) fn prune(&self, now_ms: u32) {
        let mut state = self.lock();
        state.table.hold_in_reach(now_ms);
        state.own.running_since_ms = state.own.running_since_ms.map(|since_ms| in_reach(now_ms, since_ms));
        let marks = {
            let roster = state.roster(now_ms, self.timing);
            state.table.marks(|board| !roster.keeps(board))
        };
        state.table.drop_marked(marks);
    }

    pub(crate) fn forget(&self, node: u8, now_ms: u32) -> Forgotten {
        let mut state = self.lock();
        if state.roster(now_ms, self.timing).heard(node) {
            return Forgotten::StillHeard;
        }
        let timing = self.timing;
        state.table.retain(|board| board.node_no != node || board.is_live(now_ms, timing));
        Forgotten::Removed
    }

    pub(crate) fn digest(&self, now_ms: u32) -> u64 {
        let state = self.lock();
        let roster = state.roster(now_ms, self.timing);
        let mut hasher = DefaultHasher::new();
        state.status(&roster, self.drawn_boot_epoch(), self.block_settled()).hash(&mut hasher);
        for board in state.table.iter() {
            (board.mac, board.node_no, roster.standing(board), roster.live(board)).hash(&mut hasher);
            (&board.pulse, board.ha_face).hash(&mut hasher);
        }
        for node in 0..NODE_COUNT {
            roster.split(node).hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl PresenceState {
    fn roster(&self, now_ms: u32, timing: PresenceTiming) -> Roster<'_> {
        Roster::new(&self.table, &self.own, now_ms, timing)
    }

    fn take(
        &mut self,
        decoded: &Decoded<'_>,
        delivery: Delivery,
        timing: PresenceTiming,
        counters: &ClusterCounters,
    ) -> Admitted {
        let header = &decoded.header;
        if self.mac_known && header.sender_mac == self.own.mac {
            bump(&counters.own_frames_dropped_total);
            return Admitted::NOTHING;
        }
        if header.cluster_tag != self.own.cluster_tag {
            bump(&counters.other_cluster_frames_total);
        }
        let at = Arriving { header, source: delivery.source, now_ms: delivery.now_ms, timing };
        let taken = match &decoded.frame {
            Frame::Pulse(pulse) => self.table.take_pulse(at, pulse),
            Frame::Hello(hello) => self.table.take_hello(header, hello),
            Frame::Command(_)
            | Frame::WireNote(_)
            | Frame::ChannelSceneInfo(_)
            | Frame::LampState(_)
            | Frame::LampInfo(_)
            | Frame::InputEvent(_)
            | Frame::InputInfo(_) => self.table.take_other(header),
        };
        let speaks = !matches!(decoded.frame, Frame::Pulse(_)) && self.speaks_for_its_node(header, delivery.now_ms, timing);
        let standing = matches!(decoded.frame, Frame::Command(_) | Frame::WireNote(_)).then(|| self.command_standing(delivery.now_ms, timing));
        Admitted { taken: Some(taken), member: speaks, standing }
    }

    fn command_standing(&self, now_ms: u32, timing: PresenceTiming) -> CommandStanding {
        let owner = self.roster(now_ms, timing).wire_owner();
        CommandStanding { running: self.mode == OwnMode::Running, owns_wire: owner == WireOwner::ThisNode, wire: self.own.wire }
    }

    fn speaks_for_its_node(&self, header: &Header, now_ms: u32, timing: PresenceTiming) -> bool {
        if header.node_no == self.own.node_no || header.cluster_tag != self.own.cluster_tag {
            return false;
        }
        let roster = self.roster(now_ms, timing);
        roster.displayed(header.node_no).is_some_and(|board| {
            board.mac == header.sender_mac && board.node_no == header.node_no && roster.live(board)
        })
    }

    fn status(&self, roster: &Roster<'_>, boot_epoch: Option<u32>, settled: bool) -> OwnStatus {
        let waiting = self.mode == OwnMode::Waiting;
        let faults = roster.faults();
        OwnStatus {
            mode: self.mode,
            node: self.own.node_no,
            wire: self.own.wire,
            mac: self.mac_known.then_some(self.own.mac),
            boot_epoch,
            wire_owner: roster.wire_owner(),
            faults,
            held_by: roster.rival().filter(|_| waiting).map(|board| board.mac),
            block: block_state(self.mode, faults.split_pair, settled),
        }
    }
}

impl ClusterPresenceReadPort for ClusterPresence {
    fn own_status(&self) -> OwnStatus {
        let state = self.lock();
        let roster = state.roster(dali2rust_platform::liveness::monotonic_ms(), self.timing);
        state.status(&roster, self.drawn_boot_epoch(), self.block_settled())
    }

    fn heard_node(&self, node: u8) -> Option<NodeView> {
        let state = self.lock();
        let roster = state.roster(dali2rust_platform::liveness::monotonic_ms(), self.timing);
        roster.displayed(node).map(|board| node_view(&roster, board, self.timing))
    }

    fn ignored_board(&self, index: usize) -> Option<IgnoredView> {
        let state = self.lock();
        let roster = state.roster(dali2rust_platform::liveness::monotonic_ms(), self.timing);
        let view = roster.ignored().nth(index).map(|(board, reason)| ignored_view(&roster, board, reason));
        view
    }

    fn neighbour_names_wire(&self, node: u8, wire: u8) -> bool {
        let state = self.lock();
        let roster = state.roster(dali2rust_platform::liveness::monotonic_ms(), self.timing);
        roster.named_by_neighbour(node, wire)
    }
}

const fn block_state(mode: OwnMode, split_pair: bool, settled: bool) -> BlockState {
    match mode {
        OwnMode::Off => BlockState::Off,
        OwnMode::Standby => BlockState::Passive,
        OwnMode::Waiting => BlockState::Contested,
        _ if split_pair => BlockState::Contested,
        OwnMode::Running if settled => BlockState::Settled,
        _ => BlockState::Listening,
    }
}

fn settled_for_ms(roster: &Roster<'_>, board: &HeardBoard, timing: PresenceTiming) -> Option<u32> {
    let holding_ms = elapsed_ms(roster.now_ms, board.holding_since_ms?);
    holding_ms.checked_sub(timing.settled_after_ms())
}

fn node_view(roster: &Roster<'_>, board: &HeardBoard, timing: PresenceTiming) -> NodeView {
    NodeView {
        node: board.node_no,
        mac: board.mac,
        source: board.source,
        installation: board.pulse.installation.clone(),
        wire: board.pulse.wire,
        wire_protection: board.pulse.wire_protection,
        ha_face: board.ha_face,
        live: roster.live(board),
        age_ms: board.age_ms(roster.now_ms),
        sequence_gaps: board.sequence_gaps,
        split: roster.split(board.node_no),
        settled_for_ms: settled_for_ms(roster, board, timing),
    }
}

fn ignored_view(roster: &Roster<'_>, board: &HeardBoard, reason: IgnoredReason) -> IgnoredView {
    IgnoredView {
        mac: board.mac,
        source: board.source,
        node: board.node_no,
        cluster_name: board.pulse.cluster_name.clone(),
        installation: board.pulse.installation.clone(),
        reason,
        age_ms: board.age_ms(roster.now_ms),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HeardAt {
    now_ms: u32,
    eras: Eras,
    own_node: u8,
}

fn feed_info(
    info: &ClusterInfo,
    header: &Header,
    page: &crate::frame::InfoPage<'_>,
    heard: HeardAt,
    counters: &ClusterCounters,
) {
    let HeardAt { now_ms, eras, own_node, .. } = heard;
    let at = Arrival { node: header.node_no, own_node, mac: header.sender_mac, epoch: header.boot_epoch, now_ms };
    counters.count_page(info.take_page(at, page, eras.info));
}

const fn input_arrival(header: &Header, now_ms: u32) -> InputArrival {
    InputArrival { node: header.node_no, mac: header.sender_mac, epoch: header.boot_epoch, now_ms }
}

fn feed_inputs(inputs: &ClusterInputs, header: &Header, page: &InputInfoPage<'_>, heard: HeardAt) {
    inputs.take_page(input_arrival(header, heard.now_ms), page, heard.eras.inputs);
}

fn hear_input(
    header: &Header,
    event: InputEvent,
    admitted: &Admitted,
    heard: HeardAt,
    (counters, inputs): (&ClusterCounters, &ClusterInputs),
) -> Option<Arrived> {
    let taken = admitted.taken.filter(|_| admitted.member)?;
    if !matches!(taken, Taken::Fresh { .. } | Taken::Restarted | Taken::Late) {
        return None;
    }
    match inputs.hear_event(input_arrival(header, heard.now_ms), header.seq, &event, heard.eras.inputs) {
        Some(InputHeard::Applied) => Some(Arrived::Input(HeardInput { node: header.node_no, event, now_ms: heard.now_ms })),
        Some(InputHeard::Older) | None => {
            bump(&counters.input_events_dropped_total);
            None
        }
    }
}

fn placed_state() -> PsramBox<PresenceState> {
    // SAFETY: each field is written once below: the `MAX_HEARD_BOARDS` slots, `own`, `mode`, `mac_known`, `link`.
    unsafe {
        PsramBox::new_with(|place: &mut MaybeUninit<PresenceState>| {
            let state = place.as_mut_ptr();
            let slots = ptr::addr_of_mut!((*state).table.slots).cast::<Option<HeardBoard>>();
            for index in 0..MAX_HEARD_BOARDS {
                slots.add(index).write(None);
            }
            ptr::addr_of_mut!((*state).own).write(OwnFacts::default());
            ptr::addr_of_mut!((*state).mode).write(OwnMode::Off);
            ptr::addr_of_mut!((*state).mac_known).write(false);
            ptr::addr_of_mut!((*state).link).write(0);
        })
    }
}

fn note_run(decoded: &Decoded<'_>, taken: Taken, now_ms: u32, caches: Caches<'_>) {
    if matches!(taken, Taken::Restarted | Taken::First { .. }) {
        let header = &decoded.header;
        let (node, mac, epoch) = (header.node_no, header.sender_mac, header.boot_epoch);
        caches.lamps.note_run(LampArrival { node, mac, epoch, now_ms });
        caches.inputs.note_run(InputArrival { node, mac, epoch, now_ms });
    }
}
