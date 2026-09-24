@stage-R4
Feature: Virtual lamp PATCH metadata and inherited type/colour

  @id:VL-020
  Scenario: PATCH updates metadata without touching runtime state
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And a successful target-state script for level 100 on short address 0
    When I PUT JSON {"power":"on","level":100} to "/api/v1/adapters/0/virtual-lamps/1/target-state"
    Then the response status should be 200
    When I PATCH JSON {"name":"Kitchen"} to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 200
    And the JSON field "name" should be "Kitchen"
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the JSON field "name" should be "Kitchen"
    And the JSON pointer "/state/level" should be 100
    And all scripted DALI exchanges should be consumed without errors

  @id:VL-022
  Scenario: PATCH rejects runtime state fields
    When I PATCH JSON {"state":{"level":200}} to "/api/v1/adapters/0/virtual-lamps/12"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

  @id:VL-023
  Scenario: PATCH rejects unknown field
    When I PATCH JSON {"foo":1} to "/api/v1/adapters/0/virtual-lamps/12"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:VL-024
  Scenario Outline: PATCH rejects binding, capabilities and derived fields
    When I PATCH JSON {"<field>":<value>} to "/api/v1/adapters/0/virtual-lamps/12"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

    Examples:
      | field                 | value |
      | binding               | {}    |
      | capabilities          | {}    |
      | device_type_effective | "x"   |
      | device_type_source    | "x"   |
      | color_mode_effective  | "x"   |
      | color_mode_source     | "x"   |

  @id:VL-025
  Scenario: A lamp inherits type, colour mode and capabilities from its bound device
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the JSON field "color_mode_effective" should be "cct"
    And the JSON field "color_mode_source" should be "discovered"
    And the JSON pointer "/capabilities/cct" should be true
    And the JSON pointer "/capabilities/rgb" should be false
    When I PATCH JSON {"color_mode_override":"rgb"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the JSON field "color_mode_effective" should be "rgb"
    And the JSON field "color_mode_source" should be "manual_override"
    And the JSON pointer "/capabilities/rgb" should be true
    And the JSON pointer "/capabilities/cct" should be true

  @id:VL-026
  Scenario Outline: PATCH rejects the retired lamp-level declared fields
    When I PATCH JSON {"<field>":"cct"} to "/api/v1/adapters/0/virtual-lamps/12"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

    Examples:
      | field               |
      | declared_type       |
      | declared_color_mode |
