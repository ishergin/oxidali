@stage-I5
Feature: WebSocket subscribe and event envelope
  As the embedded web UI
  I want one push connection carrying the resources of the screen I am on
  So that state is live without polling every socket the controller has

  @id:WS-001
  Scenario: The WebSocket endpoint upgrades without mutating state
    When I open a WebSocket connection
    Then the WebSocket connection should be established
    And the DALI mock transport should have received 0 forward frame

  @id:WS-007
  Scenario: The server greets an upgraded connection with a hello frame
    When I open a WebSocket connection
    Then the first WebSocket frame should have op "hello"
    And the hello frame should advertise protocol 1
    And the hello frame should list exactly the v1 channels

  @id:WS-002
  Scenario: Subscribe acknowledges the accepted channels
    Given an open WebSocket connection
    When the WebSocket client subscribes to "virtual_lamps,groups"
    Then the WebSocket client should receive op "subscribed"
    And the acknowledged channels should be "virtual_lamps,groups"

  @id:WS-006
  Scenario: Subscribe merges additional channels into the acting set
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "virtual_lamps"
    When the WebSocket client subscribes to "groups,scenes"
    Then the WebSocket client should receive op "subscribed"
    And the acknowledged channels should be "virtual_lamps,groups,scenes"

  @id:WS-009
  Scenario: Duplicate subscribe is idempotent
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "virtual_lamps"
    When the WebSocket client subscribes to "virtual_lamps"
    Then the WebSocket client should receive op "subscribed"
    And the acknowledged channels should be "virtual_lamps"

  @id:WS-005
  Scenario: A client can unsubscribe from channels
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "virtual_lamps,groups"
    When the WebSocket client unsubscribes from "virtual_lamps"
    Then the WebSocket client should receive op "unsubscribed"
    And the acknowledged channels should be "groups"

  @id:WS-008
  Scenario: Subscribe with an unknown channel is rejected atomically
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "virtual_lamps"
    When the WebSocket client subscribes to "groups,settings_network"
    Then the WebSocket client should receive op "error"
    And the WebSocket error code should be "invalid_channel"
    When the WebSocket client subscribes to "virtual_lamps"
    Then the WebSocket client should receive op "subscribed"
    And the acknowledged channels should be "virtual_lamps"

  @id:WS-003
  Scenario: A runtime state frame carries the RuntimeStateContract envelope
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
    And the runtime state payload should expand the light setpoint inline
    And the runtime state payload should expand the runtime observation inline

  @id:WS-004
  Scenario: A client only receives frames for channels it subscribed to
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "scenes"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the WebSocket client should receive no frame within 400 ms

  @id:WS-010
  Scenario: Operation frames project the OperationView shape
    Given a golden control-gear discovery script for short address 0
    And an open WebSocket connection
    And the WebSocket client is subscribed to "operations"
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the WebSocket client should receive a "OperationStatusChangedEvent" frame on channel "operations"
    And the operation payload should carry a string operation_id
    And the operation payload should carry snake_case type and status
    And the operation payload should not carry a correlation_id

  @id:WS-011
  Scenario: An upgrade beyond the client cap is refused on the protocol
    Given 4 open WebSocket connections
    When I open a WebSocket connection
    Then the WebSocket client should receive op "error"
    And the WebSocket error code should be "ws_clients_exhausted"
    And the diagnostics websocket counter "upgrades_rejected_total" should be at least 1

  @id:WS-012
  Scenario: A client ping is answered with a pong
    When I open a WebSocket connection
    And the WebSocket client sends a ping
    Then the WebSocket client should receive a pong

  @id:WS-013
  Scenario: A ping is answered while the fan-out is sending
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "virtual_lamps"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    When the WebSocket client sends a ping
    Then the WebSocket client should receive a pong
    And the WebSocket client should receive a "RuntimeStateChangedEvent" frame on channel "virtual_lamps"
