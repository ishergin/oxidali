@stage-F2
Feature: DALI Command API
  As a lighting controller
  I want to send DALI commands via HTTP
  So that I can control luminaires

  @id:DIAG-102
  Scenario: Send direct arc power command
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true

  @id:DIAG-103
  Scenario: Send query with no backward frame
    Given a DALI mock transport with no response
    When I send a JSON DALI command with address 1 and command 160
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be false

  @id:DIAG-104
  Scenario: Send command with invalid body
    Given a DALI mock transport with response 200
    When I send an invalid DALI command request
    Then the response status should be 400
