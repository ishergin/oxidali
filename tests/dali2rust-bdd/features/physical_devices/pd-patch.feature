@stage-R6
Feature: Physical-device patch metadata caps

  @id:PD-027
  Scenario: PATCH accepts name and notes at the exact byte caps
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"name":"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ01","notes":"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKL"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON field "name" should be "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ01"
    And the JSON field "notes" should be "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKL"

  @id:PD-028
  Scenario: PATCH rejects a name longer than 64 bytes on an existing device
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"name":"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ01x"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:PD-029
  Scenario: PATCH rejects notes longer than 48 bytes on an existing device
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"notes":"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLM"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:PD-035
  Scenario: color_mode_override asserts a colour capability the scan denied
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON pointer "/capabilities/cct" should be true
    And the JSON pointer "/capabilities/rgb" should be false
    When I PATCH JSON {"color_mode_override":"rgb"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON pointer "/capabilities/rgb" should be true
    And the JSON pointer "/capabilities/cct" should be true

  @id:PD-036
  Scenario: Clearing color_mode_override returns capabilities to scan evidence
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"color_mode_override":"rgb"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON pointer "/capabilities/rgb" should be true
    When I PATCH JSON {"color_mode_override":null} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON pointer "/capabilities/rgb" should be false
    And the JSON pointer "/capabilities/cct" should be true

  @id:PD-060
  Scenario: A 32-character Cyrillic name is accepted, because it is exactly 64 bytes
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    When I PATCH JSON {"name":"ЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯ"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON field "name" should be "ЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯЯ"

  @id:PD-061
  Scenario: A name that is neither a string nor null is rejected and changes nothing
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    When I PATCH JSON {"name":"Desk"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    When I PATCH JSON {"name":123} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON field "name" should be "Desk"
    When I PATCH JSON {"name":null} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON field "name" should be ""

  @id:PD-062
  Scenario: A body carrying name and notes together applies both
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    When I PATCH JSON {"name":"Kitchen","notes":"top shelf"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON field "name" should be "Kitchen"
    And the JSON field "notes" should be "top shelf"

  @id:PD-063
  Scenario: A null notes clears the stored notes
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    When I PATCH JSON {"notes":"bench"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON field "notes" should be "bench"
    When I PATCH JSON {"notes":null} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON field "notes" should be absent

  @id:PD-176
  Scenario: The DT8 auto-activation repair permission defaults on and PATCHes both ways
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON boolean field "dt8_auto_activation_repair" should be true
    When I PATCH JSON {"dt8_auto_activation_repair":false} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON boolean field "dt8_auto_activation_repair" should be false
    When I PATCH JSON {"dt8_auto_activation_repair":true} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON boolean field "dt8_auto_activation_repair" should be true

  @id:PD-177
  Scenario: A non-boolean repair permission is refused
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"dt8_auto_activation_repair":"yes"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:PD-192
  Scenario: A device-type override wider than the declared set is refused, narrower is accepted
    Given a discovery script where QueryDeviceType returns MASK before QueryNextDeviceType enumerates DT6 and DT8
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually declares device types 6, 8
    When I PATCH JSON {"device_type_override":"dt6_led"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON field "device_type_effective" should be "dt6_led"
    When I PATCH JSON {"device_type_override":null} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200

  @id:PD-193
  Scenario: An override naming a type the gear never declared is refused
    Given a discovery script where short address 0 declares only DT6
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually declares device types 6
    When I PATCH JSON {"device_type_override":"dt8_color"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
