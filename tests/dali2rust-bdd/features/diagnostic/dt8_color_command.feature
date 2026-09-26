@stage-X1
Feature: DT8 Color Command Support
  As a lighting controller
  I want to control color parameters
  So that I can set color temperature and xy coordinates

  @id:DIAG-150
  Scenario: Diagnostic DT8 opcode carries its EnableDeviceType prelude
    Given a DALI mock transport with response 0
    When I send a DALI command with wire_address 1 and command 234
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the DALI mock transport should have sent forward frame 0xC108 before 0x01EA

  @id:DIAG-151
  Scenario: The prelude and its extended frame are one transaction
    Given a DALI mock transport with response 0
    When I send a DALI command with wire_address 1 and command 234
    Then the response status should be 200
    And forward frame 0xC108 should be immediately followed by 0x01EA

  @id:DIAG-152
  Scenario: A Tc step is a single control frame under the DT8 prelude
    Given a DALI mock transport with no response
    When I send a DALI command with wire_address 1 and command 232
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the DALI mock transport should have received 2 forward frames
    And forward frame 0xC108 should be immediately followed by 0x01E8

  @id:DIAG-153
  Scenario: Opcode 227 stays a DT6 configuration pair on the opcode-shaped path
    Given a DALI mock transport with no response
    When I send a DALI command with wire_address 1 and command 227
    Then the response status should be 200
    And the DALI mock transport should have received 3 forward frames
    And forward frame 0xC106 should be immediately followed by 0x01E3
    And forward frame 0x01E3 should be immediately followed by 0x01E3
