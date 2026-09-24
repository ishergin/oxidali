@stage-R5
Feature: Group metadata PATCH

  @id:GRP-020
  Scenario: Group metadata PATCH updates registry without DALI frames
    Given group 7 exists with name "Old group"
    When I PATCH JSON {"name":"Kitchen group"} to "/api/v1/adapters/0/groups/7"
    Then the response status should be 200
    And the JSON field "name" should be "Kitchen group"
    When I send a GET request to "/api/v1/adapters/0/groups/7"
    Then the JSON field "name" should be "Kitchen group"
    And the DALI mock transport should have received 0 forward frame

  @id:GRP-021
  Scenario Outline: Group PATCH rejects derived and identity fields
    When I PATCH JSON {"<field>":<value>} to "/api/v1/adapters/0/groups/7"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

    Examples:
      | field                | value |
      | capabilities_summary | "led" |
      | dirty                | false |
      | member_count_desired | 5     |
      | member_count_applied | 5     |
      | group_id             | 7     |
