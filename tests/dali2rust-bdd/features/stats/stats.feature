@stage-R14
Feature: Stats read model
  As an operator
  I want one compact product summary of uptime, memory, bus, DALI and operations
  So that load and headroom are readable without a serial console

  @id:STATS-001
  Scenario: Stats snapshot exposes every product block
    When I send a GET request to "/api/v1/stats"
    Then the response status should be 200
    And the JSON response should have the stats blocks
    And the stats heap fields should be null on the host build
    And the stats network block should be null on the host build

  @id:STATS-003
  Scenario: The stats DALI block counts what the worker executed
    Given a DALI mock transport with response 200
    And I remember the stats DALI commands executed total
    When I send a JSON raw command with frame 64766 and expects_backward false
    Then the response status should be 200
    And the stats DALI commands executed total should have increased

  @id:STATS-006
  Scenario: The stats DALI block reports wire load and foreign frames
    When I send a GET request to "/api/v1/stats"
    Then the response status should be 200
    And the stats DALI block should carry the wire load figures

  @id:STATS-004
  Scenario: Stats and controller summary report real uptime from the Clock port
    When I send a GET request to "/api/v1/stats"
    Then the response status should be 200
    And the stats uptime should eventually be above zero
    And the controller summary uptime should be above zero

  @id:STATS-005
  Scenario: Stats operations block reflects operation tracker counters
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/stats"
    Then the response status should be 200
    And the stats operations succeeded total should be at least 1
    And the stats operations running gauge should be 0

  @id:STATS-010
  Scenario: Stats is a pure read model and publishes nothing on the bus
    Given a DALI mock transport with response 200
    When I send a JSON raw command with frame 64766 and expects_backward false
    Then the response status should be 200
    Given I remember the diagnostics bus publish totals
    When I send a GET request to "/api/v1/stats"
    And I send a GET request to "/api/v1/stats"
    Then the response status should be 200
    And the diagnostics bus publish totals should be unchanged

  @id:STATS-002
  Scenario: Stats report the Home Assistant bridge
    When I send a GET request to "/api/v1/stats"
    Then the response status should be 200
    And the JSON pointer "/mqtt/connected" should be false
    And the JSON pointer "/mqtt/publishes_total" should be 0
    And the JSON pointer "/mqtt/publish_failures_total" should be 0
