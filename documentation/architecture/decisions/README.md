# Architecture decisions

The decisions that shape dali2rust, one record each, as they currently stand.

Scope: why the system is built this way. How it works is in the
[architecture documents](../README.md).

| ADR | Title | Status | Decision |
| --- | --- | --- | --- |
| [001](ADR-001-typed-bus-and-postcard-budget.md) | Typed bus, postcard budget | Accepted | Typed envelopes, `postcard`, 128-byte frames |
| [002](ADR-002-http-confirmation-bridge.md) | HTTP confirmation bridge | Accepted | One slot pool; `503` fast, `504` on timeout |
| [003](ADR-003-isr-owned-phy.md) | Interrupt owns the PHY | Amended by 010, 025 | Tasks reach the FSM through ISR-safe cells only |
| [004](ADR-004-bdd-black-box-boundary.md) | BDD is black-box | Accepted | External surfaces only; no test hooks |
| [005](ADR-005-declarative-contract-macro.md) | Contract macro, dispatch tables | Accepted | One declaration; append-only wire order |
| [006](ADR-006-routed-command-delivery.md) | Routed commands | Amended by 015, 021 | Owner unicast; every rejection has a listener |
| [007](ADR-007-apply-orchestrator.md) | Apply orchestrator | Amended by 012 | Paced bulk applies; no unbounded bursts |
| 008 | — | Removed | Superseded by ADR-011 |
| [009](ADR-009-background-wire-time-budget.md) | Background wire budget | Amended by 013 | Yield per frame; ≤ 25 % of the wire |
| [010](ADR-010-isr-crate-and-profile-exception.md) | Interrupt crate boundary | Amended by 025 | `no_std` crate, linked-binary gate |
| [011](ADR-011-retire-esp32s3.md) | One board | Accepted | ESP32-P4-ETH only, wired |
| [012](ADR-012-async-chunked-config-writes.md) | Chunked config writes | Accepted | Stage, commit by bracket, answer `202` |
| [013](ADR-013-wire-priority-and-yield-granularity.md) | Wire priority, yield granularity | Amended by 016, 017 | Who yields and where, by command kind |
| [014](ADR-014-gear-model-and-second-dali-endpoint.md) | One gear model | Accepted | Shared model; host sim plays the foreign master |
| [015](ADR-015-routed-event-delivery.md) | Routed events | Amended by 021 | Route by declared kind; undeclared is silent |
| [016](ADR-016-input-devices-and-rule-engine.md) | Input devices, rule engine | Accepted | Part 103 on one transport; bounded rules from source |
| [017](ADR-017-dali-transactions-and-frame-priority.md) | Transactions, frame priority | Amended by 020 | Priority by purpose; §9.2 transactions |
| [018](ADR-018-controller-redundancy.md) | Controller redundancy | Accepted | DiiA 351 arbitration; HTTP config pull |
| [019](ADR-019-standby-dali-endpoint.md) | Host standby endpoint | Deferred | Coprocessor design; nothing built |
| [020](ADR-020-violating-backward-frame-is-an-answer.md) | Violation is an answer | Accepted | Terminal; YES but no value |
| [021](ADR-021-required-event-delivery.md) | Delivery-required events | Accepted | Bounded backoff for sole carriers of a fact |
| [022](ADR-022-rgbwaf-channels-are-srgb.md) | RGBWAF is sRGB | Accepted | sRGB above the wire, linear on it |
| [023](ADR-023-named-subscribers-and-shared-coalescing.md) | Named subscribers | Accepted | Named event subscribers; one coalescer |
| [024](ADR-024-ota-over-ethernet.md) | OTA over Ethernet | Amended by 025 | Device pulls its image; rollback unless proven |
| [025](ADR-025-phy-interrupt-above-critical-sections.md) | Interrupt above critical sections | Accepted | Level 5, execute from PSRAM, flash gate |
| [026](ADR-026-dt8-colour-activation.md) | DT8 colour activation | Accepted | Arc command or `ACTIVATE`; no `ACTIVATE` before a `DAPC` |
| [027](ADR-027-dtr-operand-proof-and-readback-outcomes.md) | DTR proof, read-back outcomes | Accepted | Arm, prove, act; three read-back outcomes |
| [028](ADR-028-task-stacks-in-psram-on-the-xip-image.md) | Stacks and Rust heap in PSRAM | Accepted | On the XIP image only mapping flash keeps a stack internal; Rust objects from 256 B in PSRAM |
| [029](ADR-029-network-buffers-in-psram-and-a-measured-receive-ring.md) | Network buffers in PSRAM | Accepted | lwIP and received frames in PSRAM; mailbox holds the window; ring sized by its drop counter |

A new record takes the next number and the same sections (`Status`, `Date`, `Context`,
`Decision`, `Consequences`). A later decision that changes an earlier one is merged into
that record and named in its status line.
