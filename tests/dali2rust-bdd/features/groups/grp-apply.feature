@stage-R5
Feature: Groups apply

  @id:GRP-061
  Scenario: Apply with no diff returns matrix synchronously
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the response status should be 200
    And the JSON field "operation_id" should be absent
    And the DALI mock transport should have received 0 forward frame

  @id:GRP-066
  Scenario: Apply paces a diff larger than the bus queues through the orchestrator
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in all 16 groups
    And adapter 0 desired membership includes virtual lamps 2 through 63 in all 16 groups
    And cumulative group adds for short 0 across all 16 groups are scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the response status should be 202
    And the JSON field "type" should be "group_apply"
    And the last operation eventually succeeds within the group-apply pacing budget
    And the group-apply operation result should count 16 programmed and 992 skipped outcomes
    And all scripted DALI exchanges should be consumed without errors

  @id:GRP-063
  Scenario: Apply skips unbound lamps and reports them in operation detail
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 8
    And adapter 0 desired membership includes virtual lamp 9 in group 7
    And a DALI mock transport with no response
    And adapter 0 group add for short 0 group 8 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the response status should be 202
    And the JSON field "type" should be "group_apply"
    And the JSON field "operation_id" should be non-empty
    And the last operation eventually succeeds
    And the group-apply operation result should list skipped virtual lamp 9 with reason "vl_unbound"
    And the DALI mock transport should have received 4 forward frame
    When I send a GET request to "/api/v1/adapters/0/group-membership-matrix"
    Then the group membership matrix should show virtual lamp 1 desired group 8 as true and applied group 8 as true
    And the group membership matrix should show virtual lamp 9 desired group 7 as true and applied group 7 as false
