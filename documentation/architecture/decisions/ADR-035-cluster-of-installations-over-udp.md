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
the broker a single point of failure for light between segments, added up to a bridge cycle
per frame — slower than the wire, so loop suppression would have needed a hold-off — kept
every frame inside the bus frame budget, left the standby of a pair without a session until
it took over, and rebuilt the bridge's session, last will and discovery.

What the cluster finds in place: `Origin::Cluster` and `RuntimeSource::Cluster` in the
contract, the installation and node identity of
[ADR-032](ADR-032-installation-and-node-identity.md), and the redundancy pair of
[ADR-018](ADR-018-controller-redundancy.md).

## Decision

### Nodes talk directly: UDP broadcast in one L2 segment, unsigned

One datagram is one frame of at most 1472 bytes in versioned `postcard`; the board does not
reassemble IPv4 fragments, so a larger datagram would be lost in silence. A frame of an
unknown version is dropped and counted. No broker and no session stand between two nodes,
and a frame arrives before the DALI frame that caused it has ended.

Broadcast, not multicast. There are at most eight nodes and each needs every frame, so a
group address buys nothing for them. It costs something: the board's Ethernet glue installs
no multicast MAC filter (ISSUE-139 in
[known-issues](../../product-design/known-issues.md)), and a switch with IGMP snooping and no
querier stops delivering a group after a few minutes. Multicast stays a later setting.

The frame carries no signature and never will: the network is private and the cluster
trusts its segment. What bounds an accident is elsewhere — the cluster is off until it is
switched on, its name is required, accepting commands to all lamps and to single lamps are
separate switches, and a datagram may change only what the section on datagram writes
allows.

A datagram can be lost. A command is sent twice, and the receiver drops the copy by a
window per sender keyed on the sender's MAC, its boot epoch and a sequence number. The
sender paces everything else: changes coalesce in a window, a burst is bounded, and the
periodic refreshes of different nodes fall in different phases.

The network sits behind one port trait in `dali2rust-platform`, with one implementation on
`std::net::UdpSocket` for the board and the host and a hub double for tests. The port also
names the board's MAC, which a host stack is given at composition.

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
again is another channel.

The home node keeps the record: that the channel exists, and its name with a revision.
Every participating node keeps a part: one DALI group of its wire, the flags to send and to
accept, and a copy of the name. Light never depends on the home node; while it is silent a
channel cannot be created, renamed or deleted, and nothing else.

A part's group is either an existing group linked as it is, or a free group the node takes
for the channel. A node decides that a group is free from the membership of every gear it
knows, not only its own lamps, and proves it on the wire by a presence query with a positive
control before it takes the group. Each fact has one editor: a taken group is edited only
in the channel, a linked group only on its node's group matrix.

A cluster scene is built the same way: a record on a home node, and on each node a part
naming that node's scene. A recall addresses a whole node or a channel.

A channel has no level of its own. Nodes report how many members are lit, and the channel
is on when some node was commanded on and has a lit member.

### What a datagram may change

A node's settings are written only through its own REST. A datagram changes no composition,
link, flag or gear. From announcements a node persists two things only: the copy of another
node's name at a higher revision, and — after a home node is replaced — the records of its
own block, taken back from the copies its peers hold.

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
- Wire protection, a setting. Every node that hears the same DALI frame is on one wire, and
  the wire is named after one of its nodes. One live node owns it. Only the owner executes
  incoming commands to channels, to all lamps and scene recalls, and only it forwards a
  foreign master; a command its sender already executed on that wire is executed by
  nobody. The owner holds a foreign frame's observation for tens of milliseconds, and a
  neighbour's frame with the same target and verb cancels it.

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
and Home Assistant are forwarded; rules, HCL and the foreign master are switched on by the
operator. A forwarded HCL tick keeps its background priority on the receiving node, for
which the contract gets a second cluster origin. Steps need a semantic step command and
applied facts for a step and for a stopped fade, neither of which exists today.

### The bus rules hold

The cluster worker never touches the DALI wire: what it receives it publishes as the same
semantic commands every other source uses, with the cluster as their origin. It alone owns
the network, as the bridge alone owns the broker, and no datagram travels on the bus.
Settings go through the registry worker; their changed-event only wakes the reader.

### Home Assistant sees one device

One node, the face, publishes the whole cluster through the existing bridge from the
cluster cache. A lamp keeps its `unique_id` and discovery topic, so Home Assistant keeps
its history; a channel is a new entity. When two nodes are set to be the face, the lower
number publishes.

## Consequences

- ADR-032's fourth decision changes: the cluster's sender is the board, whose MAC travels
  in the datagram header, and the bus envelope's `cluster_origin_id` is not stamped.
- The receive mailbox and the socket count in `sdkconfig.p4.defaults` grow, which only a
  flash proves. The snapshot frames of `/api/v1/stats` and `/api/v1/diagnostics` grow by a
  cluster block, and the floor of internal SRAM moves by measurement.
- Rule targets and input-device events across nodes are stages of their own and get their
  own record. The stages are in the
  [roadmap](../../product-design/roadmap.md); the module's design is in
  [`cluster`](../../product-design/runtime-modules/cluster/README.md).
- BDD drives a node through HTTP, the WebSocket and the hub double, with a scenario playing
  the neighbour in frames of the production codec. What needs two composed stacks is a
  crate integration test.
- Not promised: delivery, exactly one wire owner, and equal levels after a lost `UP` or
  `DOWN` between segments — the next absolute command brings the segments together.
