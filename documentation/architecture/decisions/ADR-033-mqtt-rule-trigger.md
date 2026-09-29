# ADR-033: A broker message can trigger a rule

Status: Accepted
Date: 2026-09-29

## Context

The rule language reserved `when mqtt "topic" [is "payload"]` and compiled nothing for it:
[ADR-016](ADR-016-input-devices-and-rule-engine.md) left MQTT-originated triggers outside
its decision. Installations already publish modes, presence and scene choices to their
broker — Home Assistant automations, other controllers, a phone — and a rule should act on
them. Three facts shape the answer: the MQTT bridge is the one broker client and holds a
session only on an active controller; the rules worker owns the compiled document and
learns about the world only from typed bus frames; and a broker replays its retained
messages to every new subscription.

## Decision

1. **Exact topic names, bounded.** A trigger names one topic and optionally one payload,
   compared byte for byte. `+` and `#` are refused where they are written: a filter would
   move matching into the engine and could subscribe the controller to traffic of any
   volume. So are an empty topic, control characters and Unicode noncharacters, which
   MQTT 3.1.1 §1.5.3 lets a broker treat as a malformed packet — it closes the session,
   so one such topic would end every session right after its SUBSCRIBE. `mqtt.publish`
   takes the same check. The topic and the payload literal are at most 48 bytes each —
   what a bus frame carries — and a document names at most eight distinct topics
   (`MAX_MQTT_TRIGGER_TOPICS`). The trigger is not an input event, so its default
   cooldown is the 200 ms of every other trigger class.
2. **The bridge stays the only broker client.** It subscribes the document's topics in
   every session, after the Home Assistant command filters, at QoS 0: a trigger acts at
   most once per message, and on a clean session QoS 1 buys nothing but an
   acknowledgement. `connected` keeps its meaning — every subscription the session wants is
   sent and acknowledged, rule topics included — so a topic added mid-session lowers it
   until its SUBACK arrives.
3. **The topic set is read, not announced.** The rules store is the one home of the
   compiled document. A read port gives the bridge the document's distinct topics and a
   generation that moves on every replacement of the document: commit, enable, recompile
   after a rename, hydration, slice reload. The bridge compares the generation on every
   turn of its loop and, when it moved, re-reads the set, subscribes what is new and
   unsubscribes what is gone within the live session. `RulesChangedEvent` does not drive
   this: it is best-effort and is not published on hydration or a slice reload, and a
   missed wake would leave the session on the wrong document until the next reconnect —
   the reason the bridge already reads its settings this way
   ([ADR-015](ADR-015-routed-event-delivery.md) §4).
4. **One delivery-required event per message.** A message on a topic the bridge subscribed
   for rules becomes one `MqttRuleMessageEvent` — topic, at most 48 payload bytes and a
   `truncated` flag — consumed by the rules worker, and it is not a Home Assistant command;
   a topic that is also a command topic is both. The message is an occurrence nothing will
   re-send, like a button press, so the event goes through `publish_required`
   ([ADR-021](ADR-021-required-event-delivery.md)): the bridge owns its thread and holds no
   deadline, and the event is the one frame of its unit of work.
5. **A subscription is not an event.** A retained message the broker delivers because of
   the subscription itself carries the retain flag (MQTT 3.1.1 §3.3.1.3) and fires nothing:
   every reconnect would replay it, and a light would change because a cable was replugged.
   A retained publish to a standing subscription arrives without the flag and fires like
   any other message. The broker client port reports the flag.
6. **A long payload never matches a literal.** Its first 48 bytes travel with `truncated`
   set: it fires `when mqtt "topic"` and never `is "payload"`.
7. **A number feeds `event.value`.** A payload that is exactly a signed decimal integer
   within 32 bits is the activation's `event.value`; any other payload leaves the value
   unevaluable, and a rule that reads it reports `partial`.
8. **No loop guard beyond the existing ones.** A message is an external occurrence and
   starts a chain at depth zero. A rule that publishes to its own trigger topic fires
   itself again: its cooldown ends the loop only when the broker round trip is shorter
   than the cooldown; otherwise the bridge's publish limiter holds the loop at one publish
   a second for as long as it runs.

### Rejected alternatives

- **Wildcard filters** — matching moves into the engine, and the subscription set is no
  longer bounded by the document.
- **A broker client in the rules worker** — two sessions, two last wills and two reconnect
  policies against one broker.
- **Topics pushed to the bridge** by `RulesChangedEvent` or by commands — a lost frame
  leaves stale subscriptions, eight topics do not fit a frame, and a fresh session needs
  the whole set anyway.
- **Firing on retained deliveries** — every reconnect replays them.

## Consequences

- A standby holds no broker session, so `when mqtt` never fires there; the controller that
  becomes active subscribes with its fresh session. A message published while no session
  exists is lost for rules.
- The broker client port gains `unsubscribe` and the retain flag of an incoming message.
  The esp-idf-svc event wrapper drops that flag, so on the device the client takes data
  events from the ESP-IDF event itself.
- The bridge counts messages on rule topics as `rule_messages_total`, not as received
  commands.
- The language and its limits are in
  [operations.md](../../product-design/runtime-modules/rules-engine/operations.md) §1.7,
  the sessions in
  [mqtt-home-assistant](../../product-design/runtime-modules/mqtt-home-assistant/README.md).
