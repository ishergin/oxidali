@stage-F2
Feature: Postcard wire message delivery through the HTTP stack
  As a system integrator
  I want postcard-encoded command envelopes to reach the DALI transport intact
  So that the worker path decodes the same wire the API accepted

  @id:BUS-013
  Scenario: DALI command envelope reaches transport intact
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 5 and command 200
    Then the response status should be 200
    And the DALI mock transport should have received 1 forward frame

  @id:BUS-014
  Scenario: Successful DALI command produces transport traffic
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the DALI mock transport should have received 1 forward frame

  @id:BUS-016
  Scenario: Multiple commands produce multiple transport frames
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    And I send a JSON DALI command with address 2 and command 100
    Then the response status should be 200
    And the DALI mock transport should have received 2 forward frame
