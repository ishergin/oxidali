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
   acknowledgement. A rule topic that a command filter already covers gets no
   subscription of its own: a broker may deliver one copy per matching subscription
   (MQTT 3.1.1 §3.3.5), and Mosquitto does. `connected` changes meaning: it was raised
   once per session when the command filters were acknowledged, and it is now evaluated
   every turn and true only while every subscription the session wants, rule topics
   included, is sent and granted. A SUBACK settles the subscription whose message id it
   carries. The flag drops while a topic added mid-session waits for its SUBACK and
   stays down while the document holds a topic the broker refused (a SUBACK of 0x80,
   counted as `subscriptions_refused_total` and logged with the topic);
   `/api/v1/controller` shows it as `home_assistant.connected`.
3. **The topic set is read, not announced.** The rules store is the one home of the
   compiled document. A read port gives the bridge the document's distinct topics and a
   generation that moves on every replacement of the document: commit, enable, recompile
   after a rename, hydration, slice reload. The bridge compares the generation on every
   turn of its loop and, when it moved, re-reads the set, subscribes what is new and
   unsubscribes what is gone within the live session. `RulesChangedEvent` does not drive
   this: it is best-effort and is not published on hydration or a slice reload, and a
   missed wake would leave the session on the wrong document until the next reconnect —
   the reason the bridge already reads its settings this way
   ([ADR-015](ADR-015-routed-event-delivery.md) §4). The set covers every rule, enabled
   or not: another rule can enable one through the engine's own bit, which the document
   never sees.
4. **One delivery-required event per message, paced per topic.** A message on a rule
   topic becomes an `MqttRuleMessageEvent` — topic, at most 48 payload bytes and a
   `truncated` flag — consumed by the rules worker, and it is not a Home Assistant command;
   a topic that is also a command topic is both. Flow control belongs to the producer
   ([ADR-007](ADR-007-apply-orchestrator.md)): a busy topic, a meter at tens of hertz,
   would flood the rules worker's funnel and the events ingress and hold the thread that
   serves Home Assistant commands. So the bridge publishes at most one event per topic per
   `RULE_MESSAGE_INTERVAL_MS` (100 ms): the first message goes at once, a later one waits
   out the interval, and a newer message replaces a waiting one — the latest wins, and the
   replaced one is counted. A published message is an occurrence nothing will re-send,
   like a button press, so its event goes through `publish_required`
   ([ADR-021](ADR-021-required-event-delivery.md)): the bridge owns its thread, and the
   event is the one frame of its unit of work.
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
8. **A message inherits depth only from a publish on its topic.** A broker message cannot
   say what caused it, and the one cause the engine knows is its own `mqtt.publish`: that
   effect is recorded in the chain under its topic at the publishing activation's depth,
   and a message on that topic within `CHAIN_WINDOW_MS` (2 s) starts an activation one
   level deeper. Every other message starts a chain of its own, so a rule on a busy
   external topic never counts its own earlier messages as links. A rule that publishes to
   its own trigger topic stops at the chain depth after five activations when the round
   trip is shorter than the window. A slower loop is not caught: the bridge's publish
   limiter, a bucket of four refilled once a second that drops the excess, ends only a
   loop shorter than a second. The price is twofold: a loop through another client that
   answers a light change with a message is invisible to the chain and runs at the pace
   of the rule's cooldown, and a message another client sends on a topic a rule published
   within the window is counted as that publish's echo. Two loops inside the controller
   escape the chain as well: a hop through a timer (`start timer` → `when timer fires` or
   `every`) starts a fresh chain at depth zero, and a `mqtt.publish` to the controller's
   own `…/set` command topic executes as a Home Assistant command whose lamp change carries
   no chain link. With a period of a second or more both run until the document changes.
9. **The bridge's own topics are not triggers.** A rule on a topic the bridge publishes
   — the state or availability of a lamp, group, scene selector or input, the
   controller's availability, a discovery config — would hear the bridge echo the
   controller's own state, a loop the chain cannot see because the echo is no
   `mqtt.publish`. The bridge knows these topics from its topic set, so it does not
   subscribe such a rule topic; it logs the topic, counts it once per session
   (`own_topics_refused_total`), lists the session's refused topics in
   `/api/v1/controller` and follows the rest of the document, and `connected` does not
   wait for it. A rule reacts to the controller's own state with `when lamp(…)`. The
   Home Assistant command topics (`…/set`) are not the bridge's own and remain triggers.

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

- `when mqtt` fires only while the bridge holds a session: the Home Assistant bridge
  enabled with a broker set, on the active controller. A standby, a disabled bridge or a
  missing broker has none; the controller that becomes active subscribes with its fresh
  session, and a message published while no session exists is lost for rules.
- The broker client port gains `unsubscribe`, the retain flag of an incoming message and
  the message id of a SUBSCRIBE, which the link hands back with its SUBACK. The esp-idf-svc
  event wrapper drops the flag and the return codes, so on the device the client takes
  data and SUBACK events from the ESP-IDF event itself.
- The stats `mqtt` block counts messages on rule topics (`rule_messages_total`), those a
  newer message replaced (`rule_messages_coalesced_total`) and those lost
  (`rule_messages_lost_total`): refused by the bus after the backoff, dropped with their
  topic when the document lost it, or left over from a session that ended, whether they
  were waiting out the interval or still queued; none of them is a received command. A
  message never outlives its session: the link stamps each one with the session it
  arrived in, and a Home Assistant command left over from an ended session is dropped
  too.
- The language and its limits are in
  [mqtt-trigger.md](../../product-design/runtime-modules/rules-engine/mqtt-trigger.md),
  the sessions in
  [mqtt-home-assistant](../../product-design/runtime-modules/mqtt-home-assistant/README.md).
