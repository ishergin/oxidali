@stage-X1
Feature: DT6 LED Command Support
  As a lighting controller
  I want to send LED-specific commands
  So that I can query gear type and control LED dimming curves

  @id:DIAG-140
  Scenario: Query gear type via JSON command
    Given a DALI mock transport with response 6
    When I send a DALI command with wire_address 1 and command 237
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the JSON DaliCommandResponse backward_frame should be 6

  @id:DIAG-141
  Scenario: Enable device type for LED
    Given a DALI mock transport with no response
    When I send a JSON DALI command with address 0 and command 193
    Then the response status should be 200
    And the DALI mock transport should have received 1 forward frame
