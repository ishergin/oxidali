@stage-R7
Feature: Target-state and scene recall stay request-scoped

  @id:OP-130
  Scenario: Virtual lamp target-state is not tracked as operation
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/virtual-lamps/1/target-state"
    Then the response status should be 200
    And the operations list should contain exactly 1 operation

  @id:OP-131
  Scenario: Group target-state is not tracked as operation
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the operations list should contain exactly 2 operations

  @id:OP-132
  Scenario: Physical device target-state is not tracked as operation
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the operations list should contain exactly 1 operation

  @id:OP-133
  Scenario: Scene recall is not tracked as operation
    Given a DALI mock transport with no response
    When I send a POST request to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 200
    When I send a GET request to "/api/v1/operations"
    Then the operations list should be empty
