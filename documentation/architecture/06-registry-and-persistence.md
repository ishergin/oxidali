# 06 Registry and Persistence

How the registry holds the controller's state, who may change it, how DALI facts become
runtime state, and how configuration survives a reboot.

Scope: registry ownership and locking, the runtime update path and its merge rules, the
slice store and hydration. Terms (physical device, virtual lamp, binding, …) are defined
in [`glossary-and-invariants.md`](../product-design/glossary-and-invariants.md); the
registry's REST resources in [`../product-design/rest-api/`](../product-design/rest-api/README.md);
flash access discipline in [08](08-dali-phy-and-transport.md); record placement and the
httpd stack in [07](07-memory-and-cores.md).

## Ownership and locking

- The registry state is one `RwLock` inside `RegistryStore`
  (`dali2rust-registry-runtime`), shared as an `Arc`.
- The registry worker is its **single writer**. One funnel inbox carries the routed
  registry commands and the DALI evidence events the registry projects, so every write
  runs on that thread. A write window is an in-memory mutation; the lock is released
  before anything is published. No second thread writes.
- Single-writer is what makes the flush safe. The persistence flush streams slices to
  flash while holding the **read** lock. ESP-IDF's `RwLock` prefers writers — a queued
  writer parks new readers — but the only thread that could queue a write is the one
  flushing, so readers keep sharing the lock; commands and events wait in the inbox.
- Everyone else reads through `RegistryReadPort` and the narrower read-port traits of
  `dali2rust-domain`. A read takes the lock and returns an owned view; no mutable
  reference or write method is reachable through a port. HTTP reads go through the
  bridge traits wired in composition; mutating routes poll lock-free apply and patch
  watches after their confirmation.
- The full `PhysicalDeviceView` is an inherent method of `RegistryStore`, on no port
  trait, so a handler holding a read port cannot build it.

## Runtime state

- Runtime fields — power, level, colour, status and failure flags, `last_seen_ms`, the
  last DAPC source, the `last_active_level` shadow — live on the physical-device record;
  virtual-lamp state is a projection through the binding.
- They change only through `RegistryRuntimeUpdateCommand` (RRUC), and the state-fanout
  projector is its only production publisher. An entry that names a virtual lamp with no
  binding is refused (`VlUnbound`).
- Every successful runtime commit publishes `RuntimeStateChangedEvent` carrying the
  committed observation; an empty or default payload is forbidden.
- Runtime fields are not persisted, and neither is operation status. After a reboot
  they converge through reads (poller, attribute reads) and observed frames.

### Update path

1. `DaliWorker` publishes typed facts: the applied target state, the runtime-status part
   of an attribute read, a scene recall. The sniffer translator publishes observed
   foreign frames.
2. The projector resolves scope through `ProjectorReadPort` — group membership, scene
   rows, bindings — and publishes RRUC.
3. The registry worker commits and publishes `RuntimeStateChangedEvent`.

- A single-target fact keeps the request's correlation id to the end, so the registry
  confirms after the commit: confirmed means committed. Fan-out entries (group members,
  scene rows), sniffer facts, read results and absence observations carry
  `CORRELATION_NONE` and are never confirmed: nobody waits for them, fan-out cannot flood
  the confirmations channel, and a borrowed correlation would turn a delivery rejection
  into the failure of someone else's operation.
- Group and scene apply diffs are expanded by the apply orchestrator from a registry
  snapshot ([ADR-007](decisions/ADR-007-apply-orchestrator.md)), never by the registry.

### Merge rules

- **Observation order, not arrival order.** A producer stamps the earliest instant at
  which its value could have been true — a read stamps its start, not its finish — on a
  monotonic clock. The projector forwards stamps and never takes one of its own.
  The registry applies a fact only if `observation_supersedes` says it is not older than
  the last winner, within a bounded contention window (60 s); an unstamped fact, or one
  outside the window, falls back to arrival order, so a long-idle record never rejects a
  fresh command. `last_seen_ms` is
  wall-clock display data and is never the ordering key. A losing fact is confirmed
  `Ok`, publishes no event, and is counted (`runtime_updates_superseded`).
- **A producer writes only what it observed.** An absent field means unknown, never
  cleared. A status observation always yields a byte (`0` is "observed, all clear"), so
  `None` can only mean "did not look". A colour-only command states no power, and the
  stored power is kept.
- **Absence is the one error the registry stores.** It is set when a producer observes
  that the gear did not answer, cleared when the gear's own status or failure bytes
  arrive, and otherwise left alone; a repeated absence verdict commits nothing. Command
  outcomes are not stored as errors: nothing observable could clear them.
- **A read reports what the gear holds, and never invents.** A colour is published whole
  or not at all. A gear with no colour capability is a complete observation that clears
  the stored colour; a colour that was not read is `None`; a bus that stayed silent is
  not a fact about the gear. `QUERY COLOUR VALUE` answers `MASK` when a value is not
  available (inactive representation, lamp off) — `0xFFFF` for a 16-bit value, `0xFF`
  for a one-byte channel — and a `MASK` answer never becomes a value (value widths:
  [09](09-dali-protocol-rules.md)). The record stores exactly one colour representation,
  the one the mode names. Commands and the sniffer observe what was requested; only a
  read observes what the gear holds, so a read is also how a record converges after a
  gear clamps to its limits or after foreign frames the sniffer cannot decode.
- **On-ness is the power field alone; a level of 0 is a level.** A lamp switched on with
  no level goes out as `GO TO LAST ACTIVE LEVEL`; the commit carries level 0 only because
  a setpoint cannot say "absent". The registry keeps `last_active_level`, the level the
  gear is known to be lit at, across the OFF that zeroes the level, and projects it on
  that recall. It is a prediction: a read repairs it, a reboot empties it, and a gear
  never seen lit has none — then the level is honestly unknown. Surfaces reporting
  brightness omit it rather than publish 0.
- **A setpoint states a colour only if `states_color()` says so.** Parsed setpoints
  always carry a colour slot, so `color.is_some()` is not the test.

### Colour capability gate (multi-member fan-out)

Group, broadcast and scene-recall fan-out project one setpoint onto different gear. The
projector strips the colour for a member whose colour capability does not accept the
requested mode (`capability_accepts_color_mode`); power and level are never filtered. A
member with no known colour capability — not yet read, or genuinely non-colour — fails
open. A human capability override is applied when the read model is projected, never
stored, so clearing it returns to the hardware's own report. Single-target paths are not
filtered: they address one device.

## Persistence

- Configuration persists as named slices (`SliceKey`, `dali2rust-platform::slice_store`)
  on the raw `storage` partition, two banks per slot, each bank with a sequence number
  and a CRC32 and its header written last: a torn write leaves the previous bank
  current. The superseded bank is erased after a successful commit, not before the next
  one. There is no filesystem on the device; on the host the same slices are files,
  written to a temporary file and renamed into place.
- The store's logic is host-compiled behind the `RawFlash` trait
  (`dali2rust-bsp::slice_store_core`); only the `esp_partition_*` binding is ESP-only.
  Its test fake has NOR semantics — a program only clears bits, only an erase sets them
  — which the file store cannot express. It does not replace a real power cut on the
  bench.
- A slice's slot is fixed arithmetic over its key, with no directory. New global slices
  and new bank regions are appended after the existing slots, because an inserted slot
  re-addresses every installed slice.
- Physical devices are stored in banks of four short addresses. The legacy
  whole-adapter slot keeps its place: hydration falls back to it when the banks hold
  nothing and the next flush rewrites the adapter as banks.
- The registry worker flushes dirty slices after a short debounce, and at once after a
  deliberate single configuration write, through one reused 1 KiB chunk buffer.
- A record that changes short address (an address change, a device replacement) dirties
  the bank it left as well as the bank it joined: flushing only the destination would
  leave a stale copy that hydration restores as a second record for the same gear.
- The rules document is persisted by the rules runtime in its own slices (three source
  banks and a manifest), written from its own single-writer worker.

### Versions

- Each slice carries its own version (`PersistenceEnvelope::new(version, …)` has no
  default). A slice whose stored version differs is rejected at hydration and rewritten
  with defaults, so a bump erases that slice on every device. Bump only the slice whose
  shape moved, and say which slices a bump resets when announcing it.
- Hydration loads physical devices before virtual lamps and drops a binding whose device
  is gone. A physical-devices bump therefore orphans every virtual-lamp binding, and
  the group matrix (expressed over bindings) goes inert: it costs an installation a
  re-bind, not a re-scan. Weigh that before adding a persisted field.
- The drop is in memory only — hydration does not mark the virtual-lamp slice dirty — so
  the stored bindings survive until the next virtual-lamp write, and a controller
  re-scanned and rebooted before any lamp edit gets them back. The re-bind is the worst
  case; marking the slice dirty would make it certain.
- A bump of the DALI settings slice resets `applicationActive` to its default (active)
  and would put a stood-down standby back on the wire after an update. Redundancy
  settings keep a slice of their own, whose reset (`enabled: false`) only switches the
  mechanism off.
- Data one read restores — live measurements, memory-bank content beyond the persisted
  attributes — lives beside `attributes` on the record and is not persisted, so it costs
  no version. It is blank after a reboot until re-read, and the provenance shown with it
  says so.
- A bump whose only purpose is to discard stored data is legitimate once per reason,
  with the reason written beside the constant; a second one needs a real mechanism.

### Hydration

- There is no hydrate worker and no `RegistryHydratedEvent`. `hydrate_from_store` runs
  once during composition, on a one-shot thread with a large stack that is joined before
  any worker starts and before HTTP is mounted. Mutating HTTP is therefore never served
  before hydration — by boot order, not by a gate. A change to the boot sequence keeps
  "hydrate joins before HTTP mounts" or adds an explicit gate.
- A standby controller reloads the slices it pulled from its peer through the same
  hydration path, on a PSRAM stack; it reads every slice first, because a task on an
  external stack must not call flash
  ([ADR-018](decisions/ADR-018-controller-redundancy.md), [07](07-memory-and-cores.md)).
