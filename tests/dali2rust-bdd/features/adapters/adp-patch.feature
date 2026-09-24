@stage-R1
Feature: Adapters patch contract

  @id:ADP-010
  Scenario: PATCH adapter updates name and enabled flag
    When I PATCH JSON {"name":"Adapter Zero","enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    And the JSON field "name" should be "Adapter Zero"
    And the JSON boolean field "enabled" should be false

  @id:ADP-011
  Scenario: PATCH adapter rejects unknown field
    When I PATCH JSON {"boom":1} to "/api/v1/adapters/0"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:ADP-012
  Scenario: PATCH adapter rejects read-only field
    When I PATCH JSON {"limits":{}} to "/api/v1/adapters/0"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"
