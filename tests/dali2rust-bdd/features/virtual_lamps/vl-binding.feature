@stage-R4
Feature: Virtual lamp binding uniqueness
  As an operator
  I want one physical device to belong to exactly one virtual lamp
  So that a light has a single entity driving it and a single name in the UI

  @id:VL-034
  Scenario: Binding a device another lamp already holds is rejected
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/2/binding"
    Then the response status should be 409
    And the JSON error should be "conflict"
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/2"
    Then the response status should be 200
    And the JSON field "binding" should be absent
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 200
    And the JSON pointer "/binding/physical_short_address" should be 0

  @id:VL-035
  Scenario: The device is free again once the lamp holding it is unbound
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I send a DELETE request to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/2/binding"
    Then the response status should be 200
    And the JSON pointer "/binding/physical_short_address" should be 0

  @id:VL-036
  Scenario: Groups adopted from one device do not follow the lamp to another
    Given adapter 0 has discovered physical devices 0 and 1
    And physical device 0 reports membership of group 3 read from the gear
    And physical device 1 reports membership of group 5 read from the gear
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I send a GET request to "/api/v1/adapters/0/group-membership-matrix"
    Then the group membership matrix should show virtual lamp 1 desired group 3 as true and applied group 3 as true
    When I send a DELETE request to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I PUT JSON {"physical_short_address":1} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I send a GET request to "/api/v1/adapters/0/group-membership-matrix"
    Then the group membership matrix should show virtual lamp 1 desired group 3 as false and applied group 3 as false
    And the group membership matrix should show virtual lamp 1 desired group 5 as true and applied group 5 as true
    When I send a GET request to "/api/v1/adapters/0/groups/3"
    Then group 3 should not be marked dirty
    When I send a GET request to "/api/v1/adapters/0/groups/5"
    Then group 5 should not be marked dirty
