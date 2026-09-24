# ADR-020: A violating backward frame is an answer

Status: Accepted
Date: 2026-08-15

## Context

IEC 62386-101 §8.2.5, in the paragraph that defines the backward window: a frame that
triggers a frame-size or bit-timing violation there "shall be interpreted as a backward
frame". §9.5.2 makes it a channel the standard uses on purpose — a bus unit whose logical
units disagree shall transmit a corrupted backward frame — and independent gear produce
the same thing without meaning to: Table 22 gives an answer a 5,5–10,5 ms start window
and Table 21 allows ±4 % on the half-bit, so several gear answering one query cannot
align, whatever their bytes.

Collapsing that case into a collision or into foreign traffic costs exactly where it
matters: `COMPARE` during discovery (many gear matching one search address is normatively
YES — keep splitting), `QUERY SHORT ADDRESS` in verification (a violation is the only
signal that more than one gear holds an address), `QUERY CONTROL GEAR PRESENT` (a present
gear must not read as absent), and a broadcast boolean probe of the whole segment.

## Decision

- **The transport classifies by what the capture decoded.** A whole foreign forward
  frame inside Table 20 is `ForeignInWindow`; after 13,4 ms it is later traffic and the
  query was unanswered (`NoAnswer`). A capture shorter than a forward frame that the
  codec cannot read — an unsupported length, a failed decode, an undecodable backward
  frame — is `CorruptedInWindow`, which means exactly §8.2.5's case. A reception that
  began and never completed is judged by Table 20: inside the window it is
  `CorruptedInWindow`, after it `NoAnswer`.
- **It is terminal, and it is not a collision.** §9.1.3 applies collision detection
  during the transmission of a forward frame; in the backward window we are not
  transmitting. So the outcome ends the exchange on the first frame whatever the retry
  budget says, and recovery belongs to the caller, which re-arms first (the verify unit
  re-reads the random address and re-establishes the search address before asking
  again). On a 24-bit frame the same outcome is `Frame24Fault::Contended`, because there
  the unit that may repeat is the sequence.
- **`DaliResponse::Violation` carries it, with two named readings.** `value()` is `None`
  — a violation has no readable content, and the diagnostic raw path reports it as a
  failure rather than manufacturing a `0x00`. `is_yes()` is `true` — something answered,
  and for a boolean query (102 §3.28 YES, §3.13 NO is silence) that is the answer. Callers
  pick the reading they mean; guessing is the defect.
- **The verify unit keeps the reason it found.** Several gear on one short address
  reports `query_short_address_multiple`, not a generic `bus_contended`.
- **The gear model decides by the count of responders**: none is `NoAnswer`, one is an
  answer, two or more is a violation, because independent gear cannot align even when
  they agree on the byte.

### Rejected alternatives

- **Mapping `Violation` to `NoAnswer`** — `COMPARE` reads NO and aborts a search that
  should continue; a present gear reads absent.
- **Mapping it to an error** — transport errors are classified by their debug text, so a
  violation would be laundered into `bus_contended`.
- **A fourth transfer outcome** — `CorruptedInWindow` already meant §8.2.5's case for one
  of its producers; a second name for one fact makes reports unreadable.
- **An `Option` plus a separate flag** — two values that must be read together.

## Consequences

- `send_raw` and its siblings return `DaliResponse`, so every call site chooses a reading.
- The broadcast bus-health probe is built on the three shapes: nobody, somebody, more than
  one.
- How violations are counted, and the merged answer that decodes as a clean byte instead,
  are in [09](../09-dali-protocol-rules.md).
- The host simulator and the gear model are the test oracle for this behaviour; on real
  hardware it appears only where several gear answer one broadcast query.
