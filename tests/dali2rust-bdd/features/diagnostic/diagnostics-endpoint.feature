@stage-X1
Feature: Runtime diagnostics endpoint
  As a bench operator
  I want one JSON snapshot of the runtime delivery and decode counters
  So that silent drop paths are observable without serial access

  @id:DIAG-030
  Scenario: Diagnostics snapshot exposes every counter block
    When I send a GET request to "/api/v1/diagnostics"
    Then the response status should be 200
    And the JSON response should have the diagnostics blocks

  @id:DIAG-031
  Scenario: Bus counters behind the snapshot are live
    Given a DALI mock transport with response 200
    And I remember the diagnostics bus commands queued total
    When I send a JSON raw command with frame 64766 and expects_backward false
    Then the response status should be 200
    And the diagnostics bus commands queued total should have increased

  @id:DIAG-032
  Scenario: Diagnostics uptime is monotonic
    Given I remember the diagnostics uptime
    When I send a GET request to "/api/v1/diagnostics"
    Then the response status should be 200
    And the diagnostics uptime should not decrease

  @id:DIAG-033
  Scenario: The wire block carries its occupancy figures
    When I send a GET request to "/api/v1/diagnostics"
    Then the response status should be 200
    And the diagnostics wire block should carry the occupancy figures
