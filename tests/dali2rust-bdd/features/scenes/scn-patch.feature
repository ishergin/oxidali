@stage-R6
Feature: Scene metadata PATCH

  @id:SCN-020
  Scenario: Scene metadata PATCH updates registry without DALI frames
    Given adapter 0 has scenes 1 and 3 configured
    When I PATCH JSON {"name":"Dinner"} to "/api/v1/adapters/0/scenes/3"
    Then the response status should be 200
    And the JSON field "name" should be "Dinner"
    When I send a GET request to "/api/v1/adapters/0/scenes/3"
    Then the JSON field "name" should be "Dinner"
    And the DALI mock transport should have received 0 forward frame

  @id:SCN-021
  Scenario Outline: Scene PATCH rejects derived and identity fields
    When I PATCH JSON {"<field>":<value>} to "/api/v1/adapters/0/scenes/3"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

    Examples:
      | field              | value |
      | row_count_included | 5     |
      | dirty              | false |
      | scene_id           | 4     |
