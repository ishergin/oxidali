@stage-I10
Feature: A broker message fires a rule

  I10-B. The MQTT bridge is the one broker client: it subscribes the rules
  document's `when mqtt` topics in its session and hands each message on them
  to the rules worker as one typed event (ADR-033). A scenario that fires a
  rule runs from a client publishing on the mock broker to the frame on the
  mock DALI transport; nothing hand-builds the event.

  @id:RULE-080
  Scenario: A matching payload fires the rule, and its broadcast reaches the wire
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When I PUT JSON {"base_revision":0,"source":"rule \"вечер\" {\n  when mqtt \"home/scene\" is \"evening\"\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the MQTT broker should eventually hold a subscription to "home/scene"
    Given the DALI mock transport trace is cleared
    When a broker client publishes "evening" on "home/scene"
    Then within 3 seconds the stats pointer "/rules/activations_total" reaches 1
    And within 3 seconds the stats pointer "/mqtt/rule_messages_total" reaches 1
    And the DALI mock transport should eventually have sent forward frame 0xFE00

  @id:RULE-081
  Scenario: Another payload wakes only the rule that takes any payload
    When I PUT JSON {"base_revision":0,"source":"rule \"a-вечер\" {\n  when mqtt \"home/scene\" is \"evening\"\n  do broadcast.off()\n}\nrule \"b-любой\" {\n  when mqtt \"home/scene\"\n  do log(\"scene\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And the DALI mock transport trace is cleared
    When a broker client publishes "morning" on "home/scene"
    Then within 3 seconds the stats pointer "/rules/activations_total" reaches 1
    When I send a GET request to "/api/v1/rules?format=json"
    Then the JSON pointer "/rules/rules/0/runtime/fire_count" should be 0
    And the JSON pointer "/rules/rules/1/runtime/fire_count" should be 1
    And the mock transport should have sent no frames

  @id:RULE-082
  Scenario: A new document moves the live session's subscriptions with it
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When I PUT JSON {"base_revision":0,"source":"rule \"вечер\" {\n  when mqtt \"home/scene\" is \"evening\"\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the MQTT broker should eventually hold a subscription to "home/scene"
    When I PUT JSON {"base_revision":1,"source":"rule \"вечер\" {\n  when mqtt \"home/mode\" is \"evening\"\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the MQTT broker should eventually hold a subscription to "home/mode"
    And the MQTT broker should eventually no longer hold a subscription to "home/scene"
    And the MQTT broker should eventually observe 1 connects

  @id:RULE-083
  Scenario: A numeric payload is the activation's event value
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When I PUT JSON {"base_revision":0,"source":"rule \"уровень\" {\n  when mqtt \"home/level\"\n  do broadcast.level(event.value)\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When a broker client publishes "128" on "home/level"
    Then the DALI mock transport should eventually have sent forward frame 0xFE80

  @id:RULE-084
  Scenario: A payload longer than the bus frame never matches a literal
    When I PUT JSON {"base_revision":0,"source":"rule \"a-точно\" {\n  when mqtt \"home/scene\" is \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n  do broadcast.off()\n}\nrule \"b-любой\" {\n  when mqtt \"home/scene\"\n  do log(\"scene\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And the DALI mock transport trace is cleared
    When a broker client publishes "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" on "home/scene"
    Then within 3 seconds the stats pointer "/rules/activations_total" reaches 1
    When I send a GET request to "/api/v1/rules?format=json"
    Then the JSON pointer "/rules/rules/0/runtime/fire_count" should be 0
    And the JSON pointer "/rules/rules/1/runtime/fire_count" should be 1
    And the mock transport should have sent no frames

  @id:RULE-085
  Scenario: A rule on a topic the bridge publishes itself gets no subscription, and the rest does
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When I PUT JSON {"base_revision":0,"source":"rule \"эхо\" {\n  when mqtt \"dali/ctl1/a0/vl/1/state\"\n  do broadcast.off()\n}\nrule \"режим\" {\n  when mqtt \"home/mode\"\n  do log(\"mode\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the MQTT broker should eventually hold a subscription to "home/mode"
    And within 3 seconds the stats pointer "/mqtt/own_topics_refused_total" reaches 1
    And the MQTT broker should never have received a subscription to "dali/ctl1/a0/vl/1/state"
