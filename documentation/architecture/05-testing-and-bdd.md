# 05 Testing and BDD

How behaviour is proved in this repository: the test layers, the black-box BDD
boundary, and the conventions every scenario, step definition and test follows.

Scope: this is the single home of the testing conventions. The BDD prefix registry
lives in [`bdd/ids-registry.md`](../product-design/bdd/ids-registry.md), running the
suite in [`tests/dali2rust-bdd/README.md`](../../tests/dali2rust-bdd/README.md), the
bench suite in [`tools/hil/README.md`](../../tools/hil/README.md). The executable
features under `tests/dali2rust-bdd/features/` are the only BDD canon; there are no
design copies of scenarios.

## Layers

| Question | Layer |
| --- | --- |
| Pure logic, mapping, validation, codec edge case | Unit test (`#[cfg(test)]`) |
| One runtime slice on the real bus path | Crate integration test (`crates/*/tests/`) |
| Behaviour observable from the composed host stack | BDD (`tests/dali2rust-bdd`) |

- Prove a behaviour at the **lowest layer that proves it without bypassing the
  production path**.
- A behaviour whose evidence is a bus payload field, the boundary of a worker cycle, a
  counter or a stub clock belongs in the owning crate's tests. A domain that needs
  registry seeding, bus taps or worker introspection moves there too; it never gets a
  BDD step file that reaches inside.
- Lower-level tests seed state through the production path: publish the event the real
  producer publishes and wait for the worker to project it, never write records
  directly. No `bdd` Cargo feature, no `pub fn bdd_*` hooks, no registry seed reach-in.
- What the host cannot model is proved on the HIL bench, which is not a merge gate
  ([`STRATEGY.md`](../../tools/hil/STRATEGY.md) §1).

## Rules for every test

- **Production builders and codecs only.** Requests, envelopes and frames come from
  `dali2rust-contracts` / `dali2rust-api` helpers (`command_envelope`,
  `parse_command_envelope`, `DaliCommandRequestBuffer`, …); never hand-rolled `postcard`
  bytes or manual re-serialisation.
- **Production composition only.** The stack under test starts through the same
  composition entry points as the firmware; mocks sit only at port boundaries; there is
  no alternate bus or HTTP implementation.
- **Take a field from its real producer.** When a consumer's behaviour depends on a
  field, at least one test obtains that field from the component that really emits it.
  A hand-built input proves the consumer and nothing about the producer.
- **Pin a decoder against foreign bytes.** A codec is tested against frames captured
  from real gear or produced by `dali2rust-gear-model`, not only against its own
  encoder: a round trip through one's own encoder shows only that two copies of one
  assumption agree.
- **Synchronise on predicates and deadlines**, never on sleeps: `wait_until`,
  `recv_with_deadline` and the counter waits of `dali2rust-test-support`. Integration
  tests and BDD never call `thread::sleep`; in crate sources every sleep carries a
  `// sleep-ok:` or `// busy-wait-ok:` marker naming the protocol timing it waits for.
- **Counters are `u32` and wrap**: assert an increment across a window, never an
  absolute value or monotonicity.
- **A negative assertion proves the stimulus arrived.** It is paired with a positive
  one showing the stimulus was processed, or it waits out at least one period of the
  producing worker's cadence; otherwise it passes before anything could have happened.
- **A counter nothing moved measured nothing.** A test that proves a mechanism shows
  that mechanism's counter moved; a run that leaves it at zero is inconclusive, not
  green.
- **A red test is fixed in the code**, never by loosening its assert.
- **Pressure is caused inside the wait.** A backpressure or required-delivery test
  publishes inside its own predicate (`publish_until_refused`), drives a burst train
  rather than an unbroken flood (`flood_in_bursts_observed`), and brackets its
  experiment with `FloodOutcome::since`, so a descheduled publisher costs a turn, not a
  red test.
- **Shared helpers live in `dali2rust-test-support`** (bus taps and frame builders,
  in-memory filesystem and slice store, waits). No per-crate copies and no new ad-hoc
  `*Harness` / `*TestStack` structs.
- **A bench fixture neutralises state it does not own**: it suspends what was already
  running (the bench's own schedules, for example) and restores it afterwards.
  Cleaning up only what the test created is not isolation. The bench's session-wide
  save and restore is a safety rule of [`STRATEGY.md`](../../tools/hil/STRATEGY.md) §4.
- No `assert!(true)` placeholders, and no tests for product targets that are not being
  implemented.

## BDD black-box boundary

Decided in [`ADR-004`](decisions/ADR-004-bdd-black-box-boundary.md).

- **Drive** the composed host stack through HTTP, and through the WebSocket endpoint
  with a real client. Foreign bus traffic enters through the transport's observed-frame
  seam (`MockDaliTransport::inject_observed_frame`), the same channel the hardware
  transport feeds.
- **Assert** only HTTP status and bodies, response DTOs, WebSocket frames, frames
  recorded by `MockDaliTransport`, and effects visible at a mocked port such as the
  slice store.
- **Never** call into `BusStackRuntime` (the World may hold it only as an inert
  keep-alive), bus channels or publishers, `RegistryStore` or read ports, confirmation
  slots, or worker counters.

## Host models and the mock transport

- The gear and control-device models keep no clock. Fades are stored but instant; the
  send-twice 100 ms window, the identification and commissioning timers and the running
  of the Part 301/303 timers (their variables are modelled) are not simulated; neither
  are power-on and system-failure behaviour, memory-bank writes and the bank-1 lock, the
  xy gamut, or Part 302/304 instances. No host test proves any of them.
- The gear model's bench-conformance allowlist (`KNOWN_DIVERGENCE`) is pinned to one
  corpus capture, never "the latest", and only shrinks: every entry states its reason,
  "not modelled yet" is not one, and an entry that no longer diverges is removed.
- The host stack serves one HTTP request at a time, like the device's single httpd task,
  so a request issued while another is blocked waits for it. A scenario orders
  concurrent work by parking a transport exchange (`the DALI transport parks the next
  exchange`), never with an HTTP barrier.
- A frame's priority is read from the settling the controller requested of
  `MockDaliTransport`, which maps to one Table 22 band — never from the gaps between
  recorded frames, which are host sleeps.
- A negative wire assertion reads the frames the mock recorded, never "the strict script
  was consumed": an exhausted script lets later frames through unanswered.
- `MockDaliTransport::clear()` drops the recorded frames and the queued, scripted and
  persistent answers but keeps the per-frame 24-bit answers (`clear_frame24_answers`
  drops those), so a scenario clears before it scripts, and clears both before scripting
  a new Part 103 walk.

## Writing scenarios

- **BDD-first.** New behaviour starts with a failing scenario; the change merges when
  the new and the existing scenarios pass. Production code that adds behaviour without
  a scenario is a merge blocker. Infrastructure-only changes (refactors, docs, CI) are
  exempt.
- **Prove the claimed profile, not only success.** A scenario that claims controller
  behaviour asserts the wire: the correct address byte for the command and the
  transport-visible frame order (for example the `ENABLE DEVICE TYPE` prelude before an
  extended command).
- **Assert whole contracts.** A scenario that checks a state DTO checks the whole
  contract [`state-contracts.md`](../product-design/rest-api/contracts/state-contracts.md)
  defines for it, not only the field the change touched.
- **Use the product vocabulary.** Scenarios name fields and values as the contracts do
  (`level`, never `brightness`, outside the MQTT / Home Assistant wire), and negative
  scenarios assert the stable codes of
  [`error-dto.md`](../product-design/rest-api/contracts/error-dto.md), never
  `internal_error` or an unlisted code.
- Gherkin, step text and step comments are English. Fixtures carry no real secrets.

## Feature tree

- `features/<resource>/`, one directory per REST resource, plus three non-resource
  layers: `diagnostic/` (the low-level `/api/v1/dali/*` surface and
  `/api/v1/diagnostics`), `system/` (boot, health, composition and cross-resource
  integration), `contracts/` (HTTP → bus → wire contracts).
- Directory names are snake_case. The allowed set is the list in
  `verify_bdd_tree_policy.sh`; a new resource adds its directory there. `dali/`, `bus/`,
  `display/` and `registry/` are forbidden, and an empty directory fails the coverage
  gate.
- Steps live in `src/steps/<domain>_steps.rs`; steps shared across resources live in
  `system_steps.rs`, `diagnostic_steps.rs` and `contracts_steps.rs`.

## Tags

| Tag | Rule |
| --- | --- |
| `@id:<PREFIX><NNN>` | On every scenario, unique across the tree. The prefix is registered in [`ids-registry.md`](../product-design/bdd/ids-registry.md); three digits and an optional lowercase variant letter follow it. |
| `@stage-<S>` | Exactly one before `Feature:`, naming the [roadmap](../product-design/roadmap.md) stage (`F*`, `R*`, `I*`, `X*`) that delivered the behaviour. A scenario adds its own only as an override for an outlier. `verify_bdd_ids.sh` holds the accepted values. |
| `@wip` | Unfinished scenario, skipped by the runner and removed as soon as it is green. Forbidden in foundation-stage (`F*`) features. |
| `@flaky`, `@diagnostic` | Informational labels that neither the runner nor any gate reads. `@flaky` does not take a scenario out of the run and is not merged without a stated justification. |

A stage is complete only when every scenario tagged with it is green with no `@wip`. A
status claim that cites `@id`s holds only if each of them exists and passes.

## Step definitions

- Every `#[given]` / `#[when]` / `#[then]` has a traceability comment directly above
  it: `// <ID> <ID> …`, the `@id`s of every scenario that uses the step, separated by
  single spaces, nothing else on the line. Every ID named there must exist as an `@id`.
- The runner is built with `fail_on_skipped()`: a step without a definition fails the
  run. There are no no-op stub steps; an unfinished scenario is tagged `@wip`.

## Gates

All run in `just verify`; `just ci` also runs the suite itself.

| Script | Holds |
| --- | --- |
| `verify_bdd_tree_policy.sh` | Allowed and forbidden top-level feature directories. |
| `verify_bdd_ids.sh` | Every scenario has an `@id`; IDs unique, prefixes registered; one file-level `@stage-*`; stage values. |
| `verify_bdd_coverage.sh` | Chains the two above; `@wip` hygiene; no empty feature directory; every ID in a step comment is an executable `@id`; an advisory report of steps whose comment omits scenarios that use them. |
| `verify_bdd_layers.sh` | The black-box boundary, over the whole BDD crate except `benches/`. |
| `verify_no_bdd_production_hooks.sh` | No `bdd` feature, `bdd_*` hooks or registry seed reach-in. |
| `verify_test_layers.sh` | No sleeps, manual `postcard` or duplicate helpers and harnesses in tests; sleep markers in crate sources. Chains `verify_duplication.sh`. |
| `verify_duplication.sh` | Clone budget and targeted anti-patterns. |
