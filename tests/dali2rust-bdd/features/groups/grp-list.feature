@stage-R5
Feature: Groups list

  @id:GRP-001
  Scenario: Group list returns configured group summaries without DALI frames
    Given adapter 0 has groups 1 and 7 configured
    When I send a GET request to "/api/v1/adapters/0/groups"
    Then the response status should be 200
    And the groups list should contain groups 1 and 7
    And the DALI mock transport should have received 0 forward frame
