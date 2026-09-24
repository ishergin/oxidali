@stage-X1
Feature: DALI command full path end-to-end
  As a system integrator
  I want a DALI command to travel from HTTP through bus to DALI worker and back
  So that the entire pipeline works correctly

  @id:SYS-005
  Scenario: Successful command traverses full path with confirmation
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the DALI mock transport should have received 1 forward frame

  @id:SYS-006
  Scenario: Failed query still traverses the command path
    Given a DALI mock transport with no response
    When I send a JSON DALI command with address 1 and command 160
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be false
    And the DALI mock transport should have received 1 forward frame

  @id:SYS-007
  Scenario: Full path works after health check
    Given a DALI mock transport with response 200
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the JSON HealthResponse status should be "ok"
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the DALI mock transport should have received 1 forward frame
