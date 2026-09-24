@stage-F3
Feature: HTTP request waits for confirmation before responding
  As a lighting controller
  I want the HTTP handler to block until the DALI worker confirmation arrives
  So that the HTTP response reflects the actual bus execution result

  @id:DALI-015
  Scenario: HTTP response includes the backward frame value from confirmation
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 160
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the JSON DaliCommandResponse backward_frame should be 200

  @id:DALI-016
  Scenario: HTTP response reflects missing backward frame
    Given a DALI mock transport with no response
    When I send a JSON DALI command with address 1 and command 160
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be false

  @id:DALI-017
  Scenario: HTTP response reflects a different backward frame value
    Given a DALI mock transport with response 42
    When I send a JSON DALI command with address 3 and command 160
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the JSON DaliCommandResponse backward_frame should be 42
