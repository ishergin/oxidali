@stage-X1
Feature: Invalid command handling
  As an API consumer
  I want proper validation of command inputs
  So that I receive clear error responses for invalid data

  @id:DIAG-400
  Scenario: Invalid wire address byte returns error
    Given a DALI mock transport with response 200
    When I send a DALI command with wire_address 160 and command 0
    Then the response status should be 400
    And the DALI mock transport should have received 0 forward frame

  @id:DIAG-401
  Scenario: Unknown command byte returns error
    Given a DALI mock transport with response 200
    When I send a DALI command with wire_address 3 and command 11
    Then the response content indicates failure

  @id:DIAG-402
  Scenario: Empty POST body returns error
    When I send a POST request to "/api/v1/dali/command" with empty body
    Then the response status should be 400
    And the DALI mock transport should have received 0 forward frame

  @id:DIAG-403
  Scenario: Malformed JSON returns error
    When I POST invalid JSON "not json" to "/api/v1/dali/command"
    Then the response status should be 400
    And the DALI mock transport should have received 0 forward frame

  @id:DIAG-404
  Scenario: Missing required fields returns error
    Given a DALI mock transport with response 200
    When I send a DALI command with missing fields
    Then the response status should be 400
    And the DALI mock transport should have received 0 forward frame
