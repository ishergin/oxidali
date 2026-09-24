@stage-I9
Feature: WebSocket log channel
  As an operator watching a controller in a distribution board
  I want the firmware log in the browser
  So that diagnosis does not require a USB cable to the one place I cannot reach

  @id:WS-050
  Scenario: The hello frame advertises the log channel
    When I open a WebSocket connection
    Then the first WebSocket frame should have op "hello"
    And the hello frame should list exactly the v1 channels

  @id:WS-051
  Scenario: Nobody subscribed to the channel means nothing is sent
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "virtual_lamps"
    When the controller logs a warning
    Then the diagnostics websocket counter "logs_lines_total" should be 0

  @id:WS-052
  Scenario: A subscriber receives the line with its level, target and sequence
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "logs"
    When the controller logs a warning
    Then the WebSocket client should receive a "LogBatch" frame on channel "logs"
    And the batch should carry a "warn" line
    And every line in the batch should carry a target and a sequence number

  @id:WS-053
  Scenario: A client that subscribes late is given the lines that predate it
    Given the controller logged a warning with nobody subscribed to "logs"
    And an open WebSocket connection
    When the WebSocket client subscribes to "logs"
    Then the WebSocket client should receive a "LogBatch" frame on channel "logs"
    And the batch should carry a "warn" line

  @id:WS-054
  Scenario: The requested level filters what the channel carries
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "logs" at level "error"
    When the controller logs a warning
    Then the WebSocket client should receive no LogBatch carrying a "warn" line

  @id:WS-057
  Scenario: An unknown level name is refused rather than silently defaulted
    Given an open WebSocket connection
    When the WebSocket client subscribes to "logs" at level "waming"
    Then the WebSocket client should receive op "error"
    And the WebSocket error code should be "invalid_value"
