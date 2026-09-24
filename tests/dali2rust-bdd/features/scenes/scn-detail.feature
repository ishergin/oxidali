@stage-R6
Feature: Scene detail

  @id:SCN-010
  Scenario: Scene detail returns metadata, included row count and dirty flag
    Given scene 3 on adapter 0 is named "Reading"
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And adapter 0 scene 3 desired row for virtual lamp 2 has level 120
    When I send a GET request to "/api/v1/adapters/0/scenes/3"
    Then the response status should be 200
    And the JSON field "name" should be "Reading"
    And the JSON numeric field "row_count_included" should be 2
    And the JSON boolean field "dirty" should be true
    And the DALI mock transport should have received 0 forward frame

  @id:SCN-011
  Scenario: Reject GET detail with invalid scene id
    When I send a GET request to "/api/v1/adapters/0/scenes/16"
    Then the response status should be 400
    And the JSON error should be "invalid_resource_id"

  @id:SCN-012
  Scenario: Reject GET detail for non-existent adapter
    When I send a GET request to "/api/v1/adapters/99/scenes/1"
    Then the response status should be 404
    And the JSON error should be "not_found"
