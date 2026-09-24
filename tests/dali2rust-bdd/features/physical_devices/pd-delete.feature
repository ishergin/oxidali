@stage-R2
Feature: Forgetting a physical device
  As an operator who took a luminaire off the wall
  I want the controller to forget it
  So that the list, the poller and every read stop carrying a device that is gone

  @id:PD-260
  Scenario: A forgotten device leaves the list
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a DELETE request to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 204
    When I send a GET request to "/api/v1/adapters/0/physical-devices"
    Then the response status should be 200
    And the JSON pointer "/physical_devices/0" should be absent

  @id:PD-261
  Scenario: Forgetting what is not there is a 404, not a silent success
    When I send a DELETE request to "/api/v1/adapters/0/physical-devices/9"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:PD-262
  Scenario: The bound lamp survives the device, unbound
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I send a DELETE request to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 204
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 200
    And the JSON pointer "/binding/physical_short_address" should be absent

  @id:PD-263
  Scenario: An out-of-range short address is refused before anything is published
    When I send a DELETE request to "/api/v1/adapters/0/physical-devices/64"
    Then the response status should be 400
    And the JSON error should be "invalid_resource_id"

  @id:PD-265
  Scenario: Forgetting a live device is temporary — the next scan re-creates it blank
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"notes":"third from the window"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON pointer "/notes" should be "third from the window"
    When I send a DELETE request to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 204
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON pointer "/notes" should be absent
