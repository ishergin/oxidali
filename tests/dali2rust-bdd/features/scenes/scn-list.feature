@stage-R6
Feature: Scenes list

  @id:SCN-001
  Scenario: Scene list returns scene summaries without DALI frames
    Given adapter 0 has scenes 1 and 3 configured
    When I send a GET request to "/api/v1/adapters/0/scenes"
    Then the response status should be 200
    And the scenes list should contain scenes 1 and 3
    And the DALI mock transport should have received 0 forward frame
