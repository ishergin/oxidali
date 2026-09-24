@stage-X1
Feature: Settings — DALI

  Controller-global bus behaviour. Both inhabitants are gear-side permissions an
  operator has once rather than once per device, which is why the primary switch
  is here and the per-device flag stays the exception: may this controller
  restore a DT8 auto-activation bit it finds clear, and may a colour write put a
  gear into normalised RGBWAF colour control.

  @id:SET-DALI-001
  Scenario: GET returns the documented default before any PATCH
    When I send a GET request to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON boolean field "dt8_auto_activation_repair" should be true

  @id:SET-DALI-002
  Scenario: PATCH switches the restore off and back on
    When I PATCH JSON {"dt8_auto_activation_repair":false} to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON boolean field "dt8_auto_activation_repair" should be false
    When I send a GET request to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON boolean field "dt8_auto_activation_repair" should be false
    When I PATCH JSON {"dt8_auto_activation_repair":true} to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON boolean field "dt8_auto_activation_repair" should be true

  @id:SET-DALI-003
  Scenario: An unknown key is refused rather than ignored
    When I PATCH JSON {"nope":1} to "/api/v1/settings/dali"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:SET-DALI-004
  Scenario: A non-boolean restore permission is refused
    When I PATCH JSON {"dt8_auto_activation_repair":"yes"} to "/api/v1/settings/dali"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SET-DALI-005
  Scenario: The RGBWAF control permission defaults on and switches
    When I send a GET request to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON boolean field "dt8_rgbwaf_control_assert" should be true
    When I PATCH JSON {"dt8_rgbwaf_control_assert":false} to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON boolean field "dt8_rgbwaf_control_assert" should be false
    And the JSON boolean field "dt8_auto_activation_repair" should be true
    When I send a GET request to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON boolean field "dt8_rgbwaf_control_assert" should be false

  @id:SET-DALI-006
  Scenario: A non-boolean RGBWAF control permission is refused
    When I PATCH JSON {"dt8_rgbwaf_control_assert":"yes"} to "/api/v1/settings/dali"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
