@stage-R16
Feature: WebSocket — the health channel
  As an operator watching the controller
  I want the live stream to say when another controller holds this one quiet
  So that a wire that takes nothing of ours is visible without polling

  While another application controller holds quiescent mode
  (IEC 62386-103 §9.9.3) no forward frame of ours leaves. REST says so in
  `/api/v1/health`; the channel carries the same body.

  @id:WS-060
  Scenario: The health channel says another controller holds quiescent mode
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "health"
    When another controller sends START QUIESCENT MODE to every unit
    Then the WebSocket client should receive a "HealthSnapshot" frame on channel "health" whose payload "quiescent" is true
    Given the WebSocket client drops the frames it has already received
    When another controller sends STOP QUIESCENT MODE to every unit
    Then the WebSocket client should receive a "HealthSnapshot" frame on channel "health" whose payload "quiescent" is false

  @id:WS-061
  Scenario: The health snapshot is the body of GET /api/v1/health
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "health"
    Then the WebSocket client should receive a "HealthSnapshot" frame on channel "health"
    And the frame payload should carry the same answer as "/api/v1/health"
