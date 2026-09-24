@stage-I5
Feature: WebSocket backpressure and client accounting
  As a controller serving several clients from one HTTP task
  I want a slow client to lose only its own view
  So that neither the bus nor another client pays for it

  @id:WS-032
  Scenario: A client that stops reading does not starve another
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given 2 open WebSocket connections subscribed to "virtual_lamps"
    And WebSocket client 1 stops reading its socket
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And WebSocket client 2 should receive a "RuntimeStateChangedEvent" frame on channel "virtual_lamps"

  @id:WS-030
  Scenario: Delivery counters account for what was sent and to whom
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "virtual_lamps"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the WebSocket client should receive a "RuntimeStateChangedEvent" frame on channel "virtual_lamps"
    And the diagnostics websocket counter "events_sent_total" should be at least 1
    And the diagnostics websocket counter "clients" should be 1

  @id:WS-046
  Scenario: Coalescing is invisible: final state on both channels, no drop notice
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "virtual_lamps"
    And the WebSocket client is subscribed to "physical_devices"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the WebSocket client should receive a "RuntimeStateChangedEvent" frame on channel "virtual_lamps"
    And the WebSocket client should receive a "RuntimeStateChangedEvent" frame on channel "physical_devices"
    And the WebSocket client should receive no frame within 300 ms

  @id:WS-031
  Scenario: A disconnected client frees its slot
    Given 4 open WebSocket connections
    When all WebSocket clients disconnect
    Then the diagnostics websocket counter "clients" should eventually be 0
    When I open a WebSocket connection
    Then the first WebSocket frame should have op "hello"
