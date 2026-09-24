@stage-F3
Feature: HTTP endpoint uses composed bus runtime
  As a system integrator
  I want HTTP endpoints to route through the bus runtime
  So that commands reach the DALI transport via the worker

  @id:SYS-017
  Scenario: DALI command goes through mock transport
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true

  @id:SYS-018
  Scenario: DALI query fails when transport has no backward response
    Given a DALI mock transport with no response
    When I send a JSON DALI command with address 1 and command 160
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be false
