@stage-F3
Feature: HTTP command enters the Commands channel
  As a lighting controller
  I want HTTP POST /api/v1/dali/command to publish a command to the DALI transport
  So that the DALI worker executes it on the bus

  @id:DALI-012
  Scenario: Successful command is forwarded to DALI transport
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the DALI mock transport should have received 1 forward frame

  @id:DALI-013
  Scenario: Command with different address reaches transport
    Given a DALI mock transport with response 100
    When I send a JSON DALI command with address 5 and command 200
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the DALI mock transport should have received 1 forward frame

  @id:DALI-014
  Scenario: Multiple commands are all forwarded in order
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    And I send a JSON DALI command with address 2 and command 128
    Then the response status should be 200
    And the DALI mock transport should have received 2 forward frames
