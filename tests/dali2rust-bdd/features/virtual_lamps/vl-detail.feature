@stage-R4
Feature: Virtual lamp detail

  @id:VL-010
  Scenario: Bound virtual lamp read exposes the full runtime-state contract
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 200
    And the virtual lamp detail should expose the full runtime-state contract
    And the JSON pointer "/binding/physical_short_address" should be 0
    And the JSON field "device_type_effective" should be "dt8_color"
    And the JSON field "device_type_source" should be "discovered"
    And the JSON field "color_mode_effective" should be "cct"
    And the JSON field "color_mode_source" should be "discovered"
    And the DALI mock transport should have received 0 forward frame

  @id:VL-011
  Scenario: Unbound virtual lamp exposes null runtime fields
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/12"
    Then the response status should be 200
    And the JSON field "binding" should be absent
    And the JSON pointer "/state/power" should be "unknown"
    And the JSON pointer "/state/value_source" should be null
    And the JSON pointer "/state/last_seen_ms" should be null
    And all virtual lamp capability flags should be false

  @id:VL-012
  Scenario: An unbound lamp inherits nothing and reports the discovered source
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/12"
    Then the response status should be 200
    And the JSON field "device_type_effective" should be "unknown"
    And the JSON field "device_type_source" should be "discovered"
    And the JSON field "color_mode_effective" should be "unknown"
    And the JSON field "color_mode_source" should be "discovered"

  @id:VL-013
  Scenario: Virtual lamp detail with id out of range returns 400
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/99"
    Then the response status should be 400
    And the JSON error should be "invalid_resource_id"
