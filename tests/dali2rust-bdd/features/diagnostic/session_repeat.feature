@stage-X1
Feature: DALI Session Repeat Semantics
  As a DALI controller
  I want configuration commands to be sent twice
  So that devices reliably receive configuration per IEC 62386-102

  @id:DIAG-210
  Scenario: Configuration command requires repeat
    Given a DALI mock transport with no response
    When I send a DALI command with wire_address 1 and command 64
    Then the response status should be 200
    And the DALI mock transport should have received 2 forward frames

  @id:DIAG-211
  Scenario: Non-configuration command does not repeat
    Given a DALI mock transport with no response
    When I send a DALI command with wire_address 255 and command 0
    Then the response status should be 200
    And the DALI mock transport should have received 1 forward frame

  @id:DIAG-212
  Scenario: Bus idle waits between repeated frames
    Given a DALI mock transport that records timestamps
    When I send a DALI command with wire_address 255 and command 32
    Then forward frames 0 and 1 should be at least 13 milliseconds apart

  @id:DIAG-213
  Scenario: An extended configuration command is a pair under one prelude
    Given a DALI mock transport with no response
    When I send a DALI command with wire_address 1 and command 224
    Then the response status should be 200
    And the DALI mock transport should have received 3 forward frames
    And forward frame 0xC106 should be immediately followed by 0x01E0
    And forward frame 0x01E0 should be immediately followed by 0x01E0
