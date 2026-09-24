@stage-R5
Feature: Group membership matrix

  @id:GRP-030
  Scenario: Matrix read returns desired and applied rows without DALI frames
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    Given a DALI mock transport with no response
    When I send a GET request to "/api/v1/adapters/0/group-membership-matrix"
    Then the response status should be 200
    And the group membership matrix should expose 16 group columns
    And the group membership matrix should show virtual lamp 1 desired group 7 as true and applied group 7 as true
    And the DALI mock transport should have received 0 forward frame

  @id:REG-030
  Scenario: Desired group membership can differ from applied membership
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    When adapter 0 desired membership excludes virtual lamp 1 from group 7
    And I send a GET request to "/api/v1/adapters/0/groups/7"
    Then the response status should be 200
    And group 7 should be marked dirty
    When I send a GET request to "/api/v1/adapters/0/group-membership-matrix"
    Then the group membership matrix should show virtual lamp 1 desired group 7 as false and applied group 7 as true
