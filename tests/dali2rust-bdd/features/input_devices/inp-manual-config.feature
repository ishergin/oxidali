@stage-R16
Feature: IEC 62386-333 manual configuration events reach the rules and the registry

  A control device reconfigured by hand (a knob, a switch on the device) announces it
  with a 24-bit manual configuration event (Part 333 §9.3). The event names the device
  by its short address and never by an instance number.

  @id:INP-100
  Scenario: A manual configuration event fires the rule of the device that sent it
    When I PUT JSON {"base_revision":0,"source":"rule \"knob\" {\n  when input device(dev=5) manual config changed\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When a 24-bit input event frame E0 B0 C5 is observed on the bus
    Then within 3 seconds the stats pointer "/rules/activations_total" reaches 1
    And the mock transport should have sent a broadcast off frame

  @id:INP-101
  Scenario: Another device's manual configuration does not fire the rule
    When I PUT JSON {"base_revision":0,"source":"rule \"knob\" {\n  when input device(dev=5) manual config changed\n  do log(\"knob\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When a 24-bit input event frame E0 B0 C6 is observed on the bus
    And a 24-bit input event frame E0 B0 C5 is observed on the bus
    Then within 3 seconds the stats pointer "/rules/activations_total" reaches 1
    When I send a GET request to "/api/v1/rules?format=json"
    Then the JSON pointer "/rules/rules/0/runtime/fire_count" should be 1

  @id:INP-102
  Scenario: A short address changed by hand fires the rules of the previous and the current address
    When I PUT JSON {"base_revision":0,"source":"rule \"a-was\" {\n  when input device(dev=5) manual config changed\n  do log(\"was\")\n}\nrule \"b-now\" {\n  when input device(dev=7) manual config changed\n  do log(\"now\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When a 24-bit input event frame E6 A2 C7 is observed on the bus
    Then within 3 seconds the stats pointer "/rules/activations_total" reaches 2
    When I send a GET request to "/api/v1/rules?format=json"
    Then the JSON pointer "/rules/rules/0/runtime/fire_count" should be 1
    And the JSON pointer "/rules/rules/1/runtime/fire_count" should be 1

  @id:INP-103
  Scenario: A frame in the manual configuration space that Part 333 reserves stays unknown
    Given the DALI mock transport trace is cleared
    When a 24-bit input event frame E4 81 C5 is observed on the bus
    Then diagnostics sniffer_translator unknown_seen should be at least 1
    And diagnostics sniffer_translator input_events_generic should be 0
    And diagnostics sniffer_translator input_lifecycle should be 0

  @id:INP-104
  Scenario: The device's record says when it announced a manual configuration
    Given the mock bus answers 24-bit query "0B FE 35" with "01"
    And the mock bus answers 24-bit query "0B 00 80" with "01"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/5"
    Then the JSON pointer "/manual_config_changed_at_ms" should be null
    When I open a WebSocket connection
    And the WebSocket client subscribes to "input"
    Then the WebSocket client should receive op "subscribed"
    When a 24-bit input event frame E0 B0 C5 is observed on the bus
    Then the WebSocket client should receive a "InputDeviceChangedEvent" frame on channel "input" whose payload "short_address" is 5
    And the JSON pointer "/manual_config_changed_at_ms" at "/api/v1/adapters/0/input-devices/5" should eventually be greater than 0
