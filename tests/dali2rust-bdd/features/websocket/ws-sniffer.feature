@stage-I5
Feature: WebSocket DALI sniffer channel
  As an operator diagnosing a bus
  I want every frame on the wire, decoded, in real time
  So that our own sequence can be compared against a foreign master's

  @id:WS-040
  Scenario: The sniffer tap stays off until somebody subscribes
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "virtual_lamps"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the diagnostics websocket counter "sniffer_records_total" should be 0

  @id:WS-041
  Scenario: Our own transmitted frames appear in the sniffer window
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "sniffer"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the WebSocket client should receive a sniffer batch
    And the sniffer batch should contain a "tx" frame named "DAPC"

  @id:WS-042
  Scenario: A foreign frame appears in the sniffer window decoded
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "sniffer"
    When a foreign DAPC frame for short address 3 level 200 is observed on the bus
    Then the WebSocket client should receive a sniffer batch
    And the sniffer batch should contain a "rx" frame named "DAPC"
    And the sniffer batch should contain a frame targeting short address 3

  @id:WS-043
  Scenario: A special command the translator discards is still shown
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "sniffer"
    When a foreign raw forward frame 0xA3 0x42 is observed on the bus
    Then the WebSocket client should receive a sniffer batch
    And the sniffer batch should contain a "rx" frame named "DTR0"

  @id:WS-044
  Scenario: A query frame is flagged so a missing answer is visible
    Given a DALI mock transport with response 200
    And an open WebSocket connection
    And the WebSocket client is subscribed to "sniffer"
    When I send a JSON raw command with frame 411 and expects_backward true
    Then the response status should be 200
    And the WebSocket client should receive a sniffer batch
    And the sniffer batch should contain a query frame

  @id:WS-045
  Scenario: Unsubscribing switches the tap back off
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "sniffer"
    When the WebSocket client unsubscribes from "sniffer"
    Then the WebSocket client should receive op "unsubscribed"
    And I remember the diagnostics websocket sniffer records total
    Given a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the diagnostics websocket sniffer records total should not have increased
