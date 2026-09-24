@stage-R1
Feature: Adapters read surface

  @id:ADP-001
  Scenario: GET adapters list returns the composed host adapters
    When I send a GET request to "/api/v1/adapters"
    Then the response status should be 200
    And the response body should contain "Main DALI"

  @id:ADP-002
  Scenario: GET adapter detail returns adapter zero
    When I send a GET request to "/api/v1/adapters/0"
    Then the response status should be 200
    And the JSON field "name" should be "Main DALI"
    And the JSON boolean field "enabled" should be true
