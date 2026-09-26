@stage-F3
Feature: DALI worker produces a confirmation after executing a command
  As a bus subscriber
  I want the DALI worker to publish a confirmation envelope after bus execution
  So that the confirmation bridge can deliver it to the waiting HTTP handler

  @id:DIAG-108
  Scenario: Successful backward frame produces Ok confirmation
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true

  @id:DIAG-109
  Scenario: Query with no backward frame produces ExecutionFailed confirmation
    Given a DALI mock transport with no response
    When I send a JSON DALI command with address 1 and command 160
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be false
