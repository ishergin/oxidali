@stage-F3
Feature: Confirmation timeout is predictable
  As a lighting controller
  I want the HTTP handler to return a 504 timeout when no confirmation arrives
  So that I know the command did not complete within the expected time

  @id:SYS-247
  Scenario: No confirmation within timeout period returns 504
    Given a DALI mock transport with response 200
    And the DALI transport blocks indefinitely
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 504
    And the response body should contain "confirmation_timeout"

  @id:SYS-248 @stage-X1
  Scenario: Confirmation timeout can be configured
    Given a bus with confirmation timeout of 500 milliseconds
    And the DALI transport blocks indefinitely
    When I send a DALI command with wire_address 2 and command 254
    Then the response arrives within 1000 milliseconds
    And the response status should be 504
