@stage-I10
Feature: The rules engine: a frame becomes light, honestly reported

  I10-B. The engine consumes the same typed events the rest of the product
  publishes — a rule is proved from an injected 24-bit frame through the
  translator, never from a hand-built event (the ISSUE-24 rule at the
  black-box layer). Every suppression is a counter, never silence.

  @id:RULE-020
  Scenario: An input frame drives a broadcast off through a rule
    Given the mock bus answers 24-bit query "0B FE 35" with "01"
    And the mock bus answers 24-bit query "0B 00 80" with "01"
    When input devices are scanned on adapter 0 and the scan succeeds
    When I PUT JSON {"base_revision":0,"source":"rule \"lights-off\" {\n  when input(dev=5, inst=0) is short_press\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When a 24-bit input event frame 0A 80 02 is observed on the bus
    Then diagnostics sniffer_translator input_events_generic should be at least 1
    And within 3 seconds the stats pointer "/rules/activations_total" reaches 1
    And the mock transport should have sent a broadcast off frame

  @id:RULE-021
  Scenario: A dry run evaluates and executes nothing
    When I PUT JSON {"base_revision":0,"source":"rule \"night\" {\n  when at 23:00\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {} to "/api/v1/rules/night/run?dry=1"
    Then the response status should be 202
    And the last operation eventually succeeds
    And within 3 seconds the stats pointer "/rules/activations_dry" reaches 1
    And the mock transport should have sent no frames

  @id:RULE-022
  Scenario: A wet run of a disabled rule fails the operation, a dry run still previews
    When I PUT JSON {"base_revision":0,"source":"rule \"night\" {\n  when at 23:00\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"enabled":false} to "/api/v1/rules/night"
    Then the response status should be 200
    When I POST JSON {} to "/api/v1/rules/night/run"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error message should be "rule_disabled"

  @id:RULE-023
  Scenario: A disabled rule is a counted suppression, not a silence
    Given the mock bus answers 24-bit query "0B FE 35" with "01"
    And the mock bus answers 24-bit query "0B 00 80" with "01"
    When input devices are scanned on adapter 0 and the scan succeeds
    When I PUT JSON {"base_revision":0,"source":"rule \"lights-off\" {\n  when input(dev=5, inst=0) is short_press\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"enabled":false} to "/api/v1/rules/lights-off"
    Then the response status should be 200
    Given the DALI mock transport trace is cleared
    When a 24-bit input event frame 0A 80 02 is observed on the bus
    Then within 3 seconds the stats pointer "/rules/suppressed_disabled" reaches 1
    And the mock transport should have sent no frames

  @id:RULE-024
  Scenario: A periodic trigger fires from the engine's own wheel
    When I PUT JSON {"base_revision":0,"source":"rule \"тик\" {\n  when every 1s\n  do log(\"t\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Then within 4 seconds the stats pointer "/rules/activations_total" reaches 2

  @id:RULE-025
  Scenario: A group switched on with no brightness fires a group rule
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    When I PUT JSON {"base_revision":0,"source":"rule \"подсветка\" {\n  when group(7) becomes any_on\n  do log(\"on\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a group off script for group 7
    When Home Assistant publishes {"state":"OFF"} on "dali/ctl1/a0/group/7/set"
    Then MQTT should have exactly 1 publish on "dali/ctl1/a0/group/7/state"
    And the MQTT payload on "dali/ctl1/a0/group/7/state" should have string field "state" = "OFF"
    Given a group last-active-level script for group 7
    When Home Assistant publishes {"state":"ON"} on "dali/ctl1/a0/group/7/set"
    Then the scripted DALI exchanges should eventually be consumed
    And MQTT should have exactly 2 publishes on "dali/ctl1/a0/group/7/state"
    And the MQTT payload on "dali/ctl1/a0/group/7/state" should have string field "state" = "ON"
    And within 3 seconds the stats pointer "/rules/activations_total" reaches 1

  @id:RULE-026
  Scenario: The rule projection reports each rule's last firing
    When I PUT JSON {"base_revision":0,"source":"rule \"night\" {\n  when at 23:00\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/rules?format=json"
    Then the JSON pointer "/rules/rules/0/runtime/fire_count" should be 0
    When I POST JSON {} to "/api/v1/rules/night/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/rules?format=json"
    Then the JSON pointer "/rules/rules/0/runtime/fire_count" should be 1
    And the JSON pointer "/rules/rules/0/runtime/last_outcome" should be "ok"
    And the JSON pointer "/rules/rules/0/runtime/last_fired_at_ms" should be greater than 0
