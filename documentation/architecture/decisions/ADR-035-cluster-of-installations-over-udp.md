# ADR-035: The cluster: installations on one Ethernet segment, channels as records

Status: Accepted; amends ADR-032
Date: 2026-11-26

## Context

Several controllers, each on its own DALI segment, should behave as one lighting system: a
group wider than one segment, a lamp commanded from any controller, one device in Home
Assistant. DALI numbers its sixteen groups and sixteen scenes per wire, so group 3 of one
controller and group 3 of another are unrelated, and a command between controllers needs a
number both of them share.

The first design forwarded group commands through the Home Assistant MQTT session. It made
the broker a single point of failure for light between segments, added a bridge cycle and a
broker hop to every frame, kept every frame inside the bus frame budget, left the standby of a pair without a session until
it took over, and rebuilt the bridge's session, last will and discovery.

What the cluster finds in place: `Origin::Cluster` and `RuntimeSource::Cluster` in the
contract, the installation and node identity of
[ADR-032](ADR-032-installation-and-node-identity.md), and the redundancy pair of
[ADR-018](ADR-018-controller-redundancy.md).

## Decision

### Nodes talk directly: UDP broadcast in one L2 segment, unsigned

One datagram is one frame of at most 1472 bytes in versioned `postcard`; the board does not
reassemble IPv4 fragments, so a larger datagram would be lost in silence. A frame of an
unknown version is dropped and counted. No broker and no session stand between two nodes.

Broadcast, not multicast. There are at most eight nodes and each needs every frame, so a
group address buys nothing for them. It costs something: the board's own Ethernet glue
installs no multicast MAC filter, and a switch with IGMP snooping and no querier stops
delivering a group after a few minutes. Multicast stays a later setting.

The frame carries no signature and never will: the network is private and the cluster
trusts its segment. What bounds an accident is elsewhere — the cluster is off until it is
switched on, its name is required, accepting commands to all lamps and to single lamps are
separate switches, and a datagram may change only what the section on datagram writes
allows.

A datagram can be lost. A command is sent twice, and the receiver drops the copy by a
window per sender keyed on the sender's MAC, its boot epoch and a sequence number. The
sender paces everything else: changes coalesce in a window, a burst is bounded, and the
periodic refreshes of different nodes fall in different phases.

Every few seconds a node sends a pulse: that it is alive, its cluster name, its
installation, its wire and its protocol version. Liveness, the owner of a wire and the
holder of a node number are all decided from pulses.

The network sits behind one port trait in `dali2rust-platform`, with one implementation on
`std::net::UdpSocket` for the board and the host and a hub double for tests. The sender's
MAC is the one the network link port already reports, the source of `node_id`; a host or
hub stack that takes part composes a link that reports one.

### A cluster node is an installation

A cluster has at most eight nodes, numbered 0 to 7. A node is an installation in the sense
of ADR-032 — one controller or a redundancy pair — and the two boards of a pair share its
number. A board stays a board; `node_id` keeps naming it.

Two boards that claim one node number are told apart by the installation their pulse
carries. The same installation is a pair: the board that transmits is the node, and two
transmitting boards are a split pair, which every peer shows. Another installation is a
configuration error: a board that hears its number held does not start, and when both
already run the lower MAC stays, so every node decides alike. A number whose holder has
missed three pulses is free, which is how a replacement board takes its place.

The standby listens and keeps the cache warm; it neither transmits nor executes.

### One lamp space, no shared store

Every virtual lamp of a node is a cluster lamp, addressed by node, adapter and lamp; the
owning node executes a command to it as a command to its own virtual lamp, and short
addresses never leave a node.

No node stores another node's lamps. Each node projects its own state as it does today and
announces it; the others keep a volatile cache in PSRAM that is not the registry and is
never persisted. A value that has not been heard is unknown, not zero.

### A channel is a record with a home node

A channel is the cluster's group. Its number, 0 to 127, comes from a block of sixteen per
node and is issued only by the node that owns the block — the channel's home node — which
is what makes a number unique without a shared store. The record carries a birth mark, a
generation and a random part, that travels in every command: a number deleted and issued
again is another channel. A node number cannot change while the node is home to a record,
and no number is issued while two boards hold one node number; without both a block would
have two owners.

The home node keeps the record: that the channel exists, and its name with a revision.
Every participating node keeps a part: one DALI group of its wire, the flags to send and to
accept, and a copy of the name. Light never depends on the home node; while it is silent a
channel cannot be created, renamed or deleted, and nothing else.

A part's group is either an existing group linked as it is, or a free group the node takes
for the channel. A node decides that a group is free from the membership of every gear it
knows, not only its own lamps, and checks it on the wire by a presence query with a positive
control before it takes the group; a gear that is powered off and unknown to the registry
escapes both. Any group linked to a channel, taken by the node or linked as it is, is edited
in the channel, and its column on the Groups screen is closed and labelled with the
channel.

A cluster scene is built the same way: a record on a home node, and on each node a part
naming that node's scene. A recall addresses a whole node or a channel.

A channel has no level of its own. Nodes report how many members are lit, and the channel
is on when some node was commanded on and has a lit member.

### What a datagram may change

A node's settings are written only through its own REST. A datagram changes no composition,
link, flag or gear. From announcements a node persists two things only: the name of a
record another node is home to, at a higher revision, and — after a home node is replaced —
the records of its own block, taken back from the copies its peers hold.

An edit of another node's part is relayed over HTTP by the node the browser has open: one
thread, one connection, an answer of `202` with an operation, and an outcome the neighbour
announces itself, so a request that got no answer is unknown rather than failed. A browser
writing to each node directly was refused — the router sees no request headers and serves
no `OPTIONS`, and a browser often reaches one node only — as were nodes pulling their part
from the home node, an edit carried by a datagram, and no remote edit at all.

### Loops

- A fact whose source is the cluster is never forwarded.
- A duplicate is dropped, and a node does not execute its own frame.
- Outgoing commands have a ceiling per target. A setpoint above it is delayed to the last
  one, not dropped.
- A relative verb — `UP`, `DOWN`, a step, a stopped fade — is never folded into another
  command, because two steps are two steps. It waits behind the command before it, and
  commands of one target leave in their order, so a setpoint given after steps lands last.
  A command to all lamps or to a node's scenes covers every channel, so it never leaves
  before a waiting command to a channel given before it, nor that channel's after it.
  Its own ceiling, 50 ms, is above the fastest key-hold repeat a Part 301 button may have
  (`Trepeat` 100 ms), so a held key never waits for its ceiling, only for a copy in
  flight; at most four relative verbs wait per target, and one beyond them is dropped,
  counted (`relative_verbs_dropped_total`) and refused to a caller with `cluster_busy`,
  never sent late. A relative verb the sender drops for a full table, a mode change or a
  send that fails is counted there too. Only a setpoint, preset or scene recall waiting
  last is replaced by a later one, and only while no command to an overlapping target was
  given after it; otherwise the later one waits behind. At most two such waits are in a
  target's line at once, so the line always keeps room for the relative verbs it admits
  and the command after them; beyond them the later one replaces again.
- Wire protection, a setting. Every node that hears the same DALI frame is on one wire, and
  the wire is named after one of its nodes. One live node owns it. Only the owner executes
  incoming commands to channels, to all lamps and scene recalls, and only it forwards a
  foreign master; a command its sender already executed on that wire is executed by
  nobody. When a neighbour shares the wire, the owner holds a foreign frame's observation
  for tens of milliseconds, and the neighbour's frame with the same target and verb cancels
  it; alone on its wire a node holds nothing.

Exactly one owner cannot be had over a network without acknowledgements, and this record
does not promise it. After the owner fails the wire executes no channel command for a few
seconds, and when pulses are lost two owners may execute one step twice; both windows are
bounded by the pulse period.

Echo credits and a fingerprint of the verb were refused: `UP` and `DOWN` have no identity,
so any such scheme either doubles steps or swallows real key presses. Protection switched
off while a foreign master is forwarded on a shared wire is a storm, so the settings refuse
that combination instead of warning about it.

### What is forwarded

Group and broadcast commands of every source and every verb. Out of the box the operator
and Home Assistant are forwarded; HCL and the foreign master are switched on by the
operator. Commands of rules become forwardable only with the rules stage: an executed fact
carries no chain depth, so a rule answering a forwarded command would loop between nodes
unseen. A forwarded HCL tick keeps on the receiving node the wire priority an HCL tick has
at home, not that of a cluster command, for which the contract gets a second cluster
origin. `UP` and `DOWN` travel as the dim command, a step as the step command and a stopped
fade by its applied fact, and the operator gives them through the `dim` verb of a group's
or a channel's target state. Missing still is the sniffer's publication of a foreign `UP`,
`DOWN` and stopped fade, which it only counts.

### The bus rules hold

The cluster worker never touches the DALI wire: what it receives it publishes as the same
semantic commands every other source uses, with the cluster as their origin. It alone owns
the network, as the bridge alone owns the broker, and no datagram travels on the bus.
Settings go through the registry worker; their changed-event only wakes the reader.

### Home Assistant sees one device

One node, the face, publishes the whole cluster through the existing bridge from the
cluster cache. A lamp is published under the `unique_id` and discovery topic it had, which
is meant to keep its history and is proven on a real Home Assistant before a node moves; a
channel is a new entity. When two nodes are set to be the face, the lower number publishes.

## Consequences

- ADR-032's fourth decision changes: the cluster's sender is the board, whose MAC travels
  in the datagram header, and the bus envelope's `cluster_origin_id` is not stamped.
- The UDP receive mailbox, today at the ESP-IDF default, and the socket count are raised in
  `sdkconfig.p4.defaults`, which only a flash proves.
- The cluster's counters are a block in `/api/v1/stats`, and its worker one more command,
  event and confirmation subscriber in `/api/v1/diagnostics`. The WebSocket snapshot ceilings carry
  them: `STATS_SNAPSHOT_CEILING_BYTES` 7936 against a measured worst case of 7935,
  `DIAGNOSTICS_SNAPSHOT_CEILING_BYTES` 8832 against 8805. The status resource goes out
  whole in one `ClusterStatus` frame, whose ceiling is 16384 against a measured worst case
  of 15857: eight nodes and 24 ignored boards, every text at its widest. The per-node
  summary of announced channel and scene info goes out in one `ClusterInfo` frame, whose
  ceiling is 2048 against a measured worst case of 1277. The floor of
  internal SRAM moves only by a census on the board.
- Input-device events and rule targets across nodes are later stages, and the rule targets
  get a record of their own. The stages are in the
  [roadmap](../../product-design/roadmap.md); the module's design is in
  [`cluster`](../../product-design/runtime-modules/cluster/README.md).
- BDD drives a node through HTTP, the WebSocket and the hub double, with a scenario playing
  the neighbour in frames of the production codec. The hub is one more surface of
  [ADR-004](ADR-004-bdd-black-box-boundary.md), as the broker side of the MQTT double is.
  What needs two composed stacks is a crate integration test.
- Not promised: delivery, exactly one wire owner, and equal levels after a lost or dropped
  `UP`, `DOWN` or step between segments — the next absolute command brings the segments
  together.
