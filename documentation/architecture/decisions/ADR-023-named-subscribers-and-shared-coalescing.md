# ADR-023: Bus subscribers have names, and the coalescing key is shared

Status: Accepted
Date: 2026-09-07

## Context

A subscriber cannot observe its own loss: the bus drops a frame before it reaches the
inbox, so the bus's overflow counter is the only record. Identified by array position,
that record means nothing without reading composition code in registration order — and
registration order is an artefact nobody pinned, so inserting one worker silently
renumbers every finding recorded against an index.

Separately, the Home Assistant bridge needs the same last-wins burst buffer the
WebSocket fan-out already has. Two copies of that buffer would drift on exactly the
property that makes it correct: survivors drain at their last write's position.

## Decision

1. **An event subscriber carries a name** from registration
   (`subscribe_events_named`, `subscribe_commands_and_events_named`) through the bus's
   subscriber counters to `/api/v1/diagnostics`, the UI and the bench report. Every
   production event subscriber is named and names are unique; test taps may stay
   anonymous. Only events carry names: that is the channel whose overflow matters, and
   the periodic diagnostics frame sizes its worst case per channel.
2. **One coalescer, one key.** The last-wins burst buffer lives in `dali2rust-api`
   (`coalesce::BurstCoalescer<T>`), and both the WebSocket fan-out and the MQTT bridge key
   it with `ws::coalesce_key`. Every state the bridge carries is WebSocket-projected and
   keyed, and a later retained state fully supersedes an earlier one. An occurrence — an
   input-device event or a rule activation — supersedes nothing and carries no key, so two
   button presses or two firings of one rule stay two frames.

## Consequences

- A production subscriber registered without a name, or with a duplicate, fails a
  composition test.
- A red bench run names the subscriber that lost frames.
- A change to the WebSocket projection moves MQTT coalescing behaviour as well; the
  coupling is deliberate and pinned by deriving the key from the table both surfaces read.
- The bridge's coalesced-frame counter measures work spared, not loss, and never feeds a
  failure count: the broker still ends with every entity's final state.
