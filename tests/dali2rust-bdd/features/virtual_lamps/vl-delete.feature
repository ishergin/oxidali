@stage-R4
Feature: Deleting a virtual lamp
  As an operator
  I want to forget a virtual lamp I created by mistake
  So that the registry, the group matrix and every scene stop carrying a row nobody wants

  @id:VL-100
  Scenario: A deleted lamp leaves the list and frees its device
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I send a DELETE request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 204
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps"
    Then the response status should be 200
    And the virtual lamps list should not contain lamp 1
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/2/binding"
    Then the response status should be 200
    And the JSON pointer "/binding/physical_short_address" should be 0

  @id:VL-101
  Scenario: Deleting a lamp that is not there is a 404, not a silent success
    When I send a DELETE request to "/api/v1/adapters/0/virtual-lamps/7"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:VL-102
  Scenario: The deleted lamp's desired group row goes with it
    Given adapter 0 has discovered physical devices 0 and 1
    And physical device 0 reports membership of group 3 read from the gear
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I send a GET request to "/api/v1/adapters/0/group-membership-matrix"
    Then the group membership matrix should show virtual lamp 1 desired group 3 as true and applied group 3 as true
    When I send a DELETE request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 204
    When I send a GET request to "/api/v1/adapters/0/group-membership-matrix"
    Then the response status should be 200
    And the group membership matrix should show virtual lamp 1 desired group 3 as false and applied group 3 as false

  @id:VL-103
  Scenario: An out-of-range lamp id is refused before anything is published
    When I send a DELETE request to "/api/v1/adapters/0/virtual-lamps/99"
    Then the response status should be 400
    And the JSON error should be "invalid_resource_id"

  @id:VL-104
  Scenario: The deleted lamp's scene rows go with it
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix desired row for virtual lamp 1 should be included with level 100 and no color
    When I send a DELETE request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 204
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 200
    And the scene matrix desired row for virtual lamp 1 should serialize excluded setpoint fields as null
