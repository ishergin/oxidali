@stage-R4
Feature: Virtual lamps list

  @id:VL-001
  Scenario: Virtual lamp list returns stable DTOs without DALI frames
    Given virtual lamp 1 has name "Lamp one"
    And virtual lamp 12 has name "Lamp twelve"
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps"
    Then the response status should be 200
    And the virtual lamps list should contain lamp 1 named "Lamp one"
    And the virtual lamps list should contain lamp 12 named "Lamp twelve"
    And the DALI mock transport should have received 0 forward frame

  @id:VL-002
  Scenario: Virtual lamp list with unknown adapter returns 404
    When I send a GET request to "/api/v1/adapters/42/virtual-lamps"
    Then the response status should be 404
    And the JSON error should be "not_found"
