@stage-X1
Feature: DALI special commands
  As an API consumer
  I want commissioning and memory special commands to use the normal command path
  So that special-command confirmations are correlated like standard commands

  @id:DIAG-407
  Scenario: Write memory location returns a backward frame
    Given the DALI transport responds with 0x2A
    When I send a DALI command with wire_address 199 and command 85
    Then the response status should be 200
    And the JSON response success should be true
    And the JSON DaliCommandResponse backward_frame should be 42
