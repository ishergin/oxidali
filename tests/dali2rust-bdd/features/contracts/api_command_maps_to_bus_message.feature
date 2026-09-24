@stage-F2
Feature: HTTP command aligns with bus message and DALI frame (contract)
  The API request becomes a CommandEnvelope on the bus and then the expected forward frame on the wire (via the production composition path).

  @id:CONT-001
  Scenario: POST command matches DirectArcPower forward frame for a short address
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    Then the mock should have sent forward frame for short address 1 direct arc level 254
