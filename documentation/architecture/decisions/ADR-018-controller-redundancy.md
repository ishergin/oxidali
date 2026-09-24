# ADR-018: Controller redundancy — a passive standby that arbitrates on the wire

Status: Accepted
Date: 2026-08-10

## Context

A standby covers exactly one failure: the controller itself. It does not cover the bus
power supply or the wiring (those need a second supply or segmentation), and a button
that works while the network is down is already local rules
([ADR-016](ADR-016-input-devices-and-rule-engine.md)). When a master dies, gear simply
holds its last level; nothing on the bus notices.

Detecting that death is the hard part. A healthy controller is legitimately silent for
hours (the poller is off by default), so silence is not evidence, and 103 §9.2.3 forbids
a multi-master application controller the `PING` that would announce it. DiiA 351 §7
already defines how a unit finds out whether a bus has an owner, and the existing
pieces make a standby small: the sniffer observes every frame, the translator turns
foreign traffic into facts without publishing commands, configuration is slice-shaped
and versioned, and the stack is generic over its DALI transport. The standby is a second
ESP32-P4-ETH running the same image; the host-based standby of
[ADR-019](ADR-019-standby-dali-endpoint.md) is deferred.

## Decision

The mechanism — probe cadence, answer table, lease, replication loop, behaviour of a
passive unit, counters — is described in
[runtime-modules/redundancy](../../product-design/runtime-modules/redundancy/README.md);
this record keeps the decisions and their reasons.

### The wire decides the role, by the standard's own arbitration

- **Roles are configured, not negotiated.** The primary behaves as a 351 type A/C unit:
  dominant, it never asks and never stands itself down. The standby behaves as a type B
  unit: it probes with `QUERY APPLICATION CONTROLLER ENABLED` at the lowest priority and
  reads any reply, a corrupted one included, as "the bus has an owner" and silence as "it
  has none". There is no election and no identity comparison.
- **The primary answers from `applicationActive`** (103 §11.6.16: YES when true, and NO
  is silence). A passive controller answers nothing, so two standbys cannot keep each
  other passive.
- **A task decides the answer and the PHY interrupt times it**
  ([08](../08-dali-phy-and-transport.md)), so a controller in the middle of a long scan
  still answers inside Table 20's window.
- **One lost frame cannot move the bus.** Takeover needs several consecutive unanswered
  probes, a probe that never reached the wire is no verdict, and a freshly booted
  controller listens before it claims anything.
- **The wire alone decides.** The arbitration worker reads no network state: losing
  Ethernet, the broker or the peer's HTTP surface cannot move the role. There is no
  shared virtual IP and no second election; clients reach each unit by its own address
  (a static DNS entry or DHCP reservation).
- **The return path is 351's**, and it survives a reboot because `applicationActive` is
  NVM.
- **A passive controller transmits nothing but the probe**, refused at the DALI worker's
  one gate, and the Home Assistant bridge, the HCL scheduler and the poller run only in
  the active role.

### Functional liveness: a lease that a wedged worker cannot renew

Arbitration proves that the answering unit is alive, not that the controller works. So
the answers depend on a short lease the supervisor renews only while the registry, the
HCL scheduler and the rules engine keep turning; a deadlocked worker stops being spoken
for, the lease lapses, the answers stop, and the standby's next probes find an unowned
bus. The failure path is a call that does not happen, which is the only kind a wedged
process cannot skip. The DALI worker is not watched: it legitimately blocks while idle
and for the length of a scan.

### State: configuration replicates, runtime state is observed

- **Runtime state is never replicated.** Levels, colour and status flags are projected
  from observed traffic by the same translator the active controller uses for foreign
  frames; a second producer for those fields would prove nothing about the real one.
- **Configuration is pulled over HTTP** by the passive unit from its peer's slice
  export, straight into its slice store; only a reload command crosses the bus. A slice
  the peer lacks is never deleted locally, and an unreachable peer changes nothing: stale
  configuration beats a dark building.
- **What transfers** is everything that describes the installation, the broker password
  included (the maintainer's decision for a trusted LAN); what never transfers, and why, is in
  [config transfer](../../product-design/rest-api/resources/config-transfer.md).
- **Why not retained MQTT:** the pair shares one `controller_id` so Home Assistant sees
  one installation, the MQTT client id derives from it, and two units on one broker would
  evict each other; slices are also far larger than the client's inbound buffer.

### Operator surfaces

- **The role is visible** on every API response, on `/api/v1/health` and
  `/api/v1/redundancy`, on the OLED and in the web UI.
- **A write to a standby is refused, not queued:** `409 controller_standby` with
  `Retry-After`, distinct from the `503` that means overload. Reads and the WebSocket are
  served in both roles.
- **Planned switchover** hands the bus over by enabling the peer first and standing down
  second, in one direction only: 103 §9.9.1 leaves a passive controller no legal way to
  send the `DISABLE`. The lights do not move.
- **Every transition is recorded** with its reason and counted.
- **Gear fallback policies** (`systemFailureLevel`, `powerOnLevel`) are a product
  resource written by the apply orchestrator. Unset means unmanaged, which is the
  default, because both default to full brightness in the gear and silently rewriting
  them would change what an installation does in a power cut.

### Rejected alternatives

- **Takeover on observed silence** — takes over an idle, healthy bus.
- **Widening the cluster axis to cover redundancy** — cluster forwards group and broadcast
  intent between segments; a forwarded short-address command lands on the wrong
  luminaire.
- **A `controller_id` tie-break** — solves a two-arbitrating-peers problem the
  specification does not have and diverges from every third-party type B device.
- **Self-reported health** — the reporting runs in the process whose health is in
  question.
- **VRRP and a shared virtual IP** — a second election that can disagree with the wire.

## Consequences

- `dali2rust-redundancy-runtime` holds the supervisor, the arbitration worker and the
  replication worker; redundancy has its own settings slice and is off by default, in
  which case the controller behaves as a single controller.
- A third-party 351 type B device on the segment participates correctly, and a
  third-party application controller can stand our standby down without knowing what it
  is.
- Input-device instances are not gated by arbitration (351 §7), so panel events keep
  flowing in both roles, and the active unit acts on them with the same replicated rules.
- Host BDD models one controller; two controllers on one wire are proved only on the
  bench.
- `takeover_after_missed` trades detection time against spurious takeovers when single
  probes go unanswered on a busy bus; the default is still the maintainer's open choice
  (ISSUE-86 in [known-issues](../../product-design/known-issues.md)).
- The REST surface: [rest-api/resources/redundancy.md](../../product-design/rest-api/resources/redundancy.md).
