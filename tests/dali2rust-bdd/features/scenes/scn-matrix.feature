@stage-R6
Feature: Scene matrix read / PATCH / PUT

  @id:SCN-030
  Scenario: Scene matrix read exposes SceneRow composition for every row
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has CCT 2700K and level 179
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 200
    And the scene matrix should expose 64 rows with virtual_lamp_id, name, capabilities and dirty
    And the scene matrix desired row for virtual lamp 1 should be included with level 179 and color_mode "cct"
    And the scene matrix desired row for virtual lamp 0 should serialize excluded setpoint fields as null
    And scene matrix rows should not expose RuntimeObservation fields
    And the DALI mock transport should have received 0 forward frame

  @id:SCN-040
  Scenario: Scene matrix PATCH records add, update and remove without DALI frames
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 2 has level 100
    And adapter 0 scene 3 desired row for virtual lamp 1 has CCT 2700K and level 100
    When I PATCH JSON {"rows":[{"virtual_lamp_id":2,"desired":{"included":false,"power":null,"level":null,"color_mode":null,"color_temperature_kelvin":null,"xy":null,"rgb":null}},{"virtual_lamp_id":1,"desired":{"included":true,"power":"on","level":180,"color_mode":"cct","color_temperature_kelvin":2700,"xy":null,"rgb":null}},{"virtual_lamp_id":3,"desired":{"included":true,"power":"on","level":254,"color_mode":null,"color_temperature_kelvin":null,"xy":null,"rgb":null}}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix desired row for virtual lamp 2 should serialize excluded setpoint fields as null
    And the scene matrix desired row for virtual lamp 1 should be included with level 180 and color_mode "cct"
    And the scene matrix desired row for virtual lamp 3 should be included with level 254 and no color
    And the DALI mock transport should have received 0 forward frame

  @id:SCN-041
  Scenario: Scene matrix PATCH supports level and xy color for an xy-capable lamp
    Given adapter 0 has a discovered and bound xy-capable virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has CCT 2700K and level 100
    When I PATCH JSON {"rows":[{"virtual_lamp_id":1,"desired":{"included":true,"power":"on","level":200,"color_mode":"xy","xy":{"x":0.42,"y":0.38},"color_temperature_kelvin":null,"rgb":null}}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix desired row for virtual lamp 1 should be included with level 200 and color_mode "xy"
    And the scene matrix desired row for virtual lamp 1 should have inactive cct and rgb fields null
    And the DALI mock transport should have received 0 forward frame

  @id:SCN-042
  Scenario: Scene matrix PATCH rejects RuntimeObservation fields in desired
    When I PATCH JSON {"rows":[{"virtual_lamp_id":1,"desired":{"included":true,"power":"on","level":100,"status":"ok"}}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

  @id:SCN-043
  Scenario: Scene matrix PATCH rejects unsupported capability and invalid level
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PATCH JSON {"rows":[{"virtual_lamp_id":1,"desired":{"included":true,"power":"on","level":100,"color_mode":"rgb","rgb":{"r":255,"g":120,"b":40},"color_temperature_kelvin":null,"xy":null}}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 422
    And the JSON error should be "unsupported_capability"
    When I PATCH JSON {"rows":[{"virtual_lamp_id":1,"desired":{"included":true,"power":"on","level":255,"color_mode":null,"color_temperature_kelvin":null,"xy":null,"rgb":null}}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SCN-044
  Scenario: Scene matrix PATCH rejects included false with non-null setpoint fields
    When I PATCH JSON {"rows":[{"virtual_lamp_id":1,"desired":{"included":false,"power":null,"level":100,"color_mode":null,"color_temperature_kelvin":null,"xy":null,"rgb":null}}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SCN-045
  Scenario Outline: Scene matrix PATCH rejects derived row fields
    When I PATCH JSON {"rows":[{"virtual_lamp_id":1,"desired":{"included":false,"power":null,"level":null,"color_mode":null,"color_temperature_kelvin":null,"xy":null,"rgb":null},"<field>":<value>}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

    Examples:
      | field        | value               |
      | applied      | {"included":true}   |
      | capabilities | {"brightness":true} |
      | dirty        | true                |
      | name         | "Kitchen"           |

  @id:SCN-046
  Scenario: Scene matrix PATCH stores all six channels for an rgbwaf-capable lamp
    Given adapter 0 has a discovered and bound rgbwaf-capable virtual lamp 1 on physical device 0
    When I PATCH JSON {"rows":[{"virtual_lamp_id":1,"desired":{"included":true,"power":"on","level":200,"color_mode":"rgbwaf","rgbwaf":{"r":10,"g":20,"b":30,"w":40,"a":50,"f":60},"color_temperature_kelvin":null,"xy":null,"rgb":null}}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix desired row for virtual lamp 1 should be included with level 200 and color_mode "rgbwaf"
    And the scene matrix desired row for virtual lamp 1 should have rgb 10,20,30 and waf 40,50,60
    And the DALI mock transport should have received 0 forward frame

  @id:SCN-047
  Scenario: Scene matrix PATCH rejects a partial six-channel colour
    Given adapter 0 has a discovered and bound rgbwaf-capable virtual lamp 1 on physical device 0
    When I PATCH JSON {"rows":[{"virtual_lamp_id":1,"desired":{"included":true,"power":"on","level":200,"rgbwaf":{"r":10,"g":20,"b":30,"w":40,"a":50}}}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SCN-048
  Scenario: Scene matrix PATCH rejects the xy origin no gear can hold
    Given adapter 0 has a discovered and bound xy-capable virtual lamp 1 on physical device 0
    When I PATCH JSON {"rows":[{"virtual_lamp_id":1,"desired":{"included":true,"power":"on","level":200,"color_mode":"xy","xy":{"x":0.0,"y":0.0},"color_temperature_kelvin":null,"rgb":null}}]} to "/api/v1/adapters/0/scenes/3/matrix"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SCN-050
  Scenario: Scene matrix PUT replaces full desired matrix
    Given adapter 0 scene 3 desired row for virtual lamp 5 has level 90
    When I PUT a complete scene 3 matrix with virtual lamp 1 at level 210 and 63 excluded rows
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix desired row for virtual lamp 1 should be included with level 210 and no color
    And the scene matrix desired row for virtual lamp 5 should serialize excluded setpoint fields as null
    And the DALI mock transport should have received 0 forward frame

  @id:SCN-051
  Scenario: Reject PUT matrix with incorrect number of rows
    When I PUT JSON {"rows":[{"virtual_lamp_id":0,"desired":{"included":false,"power":null,"level":null,"color_mode":null,"color_temperature_kelvin":null,"xy":null,"rgb":null}}]} to "/api/v1/adapters/0/scenes/1/matrix"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:REG-031
  Scenario: Scene desired state can differ from applied state
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    When adapter 0 scene 3 desired row for virtual lamp 1 changes level to 180
    And I send a GET request to "/api/v1/adapters/0/scenes/3"
    Then the response status should be 200
    And the JSON boolean field "dirty" should be true
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix applied row for virtual lamp 1 should have included true and level 100
    And the scene matrix desired row for virtual lamp 1 should be included with level 180 and no color
