@stage-R2
Feature: Physical device direct target state

  @id:PD-040
  Scenario: Direct target state drives brightness, off and cct on the wire
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And all scripted DALI exchanges should be consumed without errors
    And the physical device 0 state level should eventually be 180
    Given a power-off target-state script for short address 0
    When I PUT JSON {"power":"off"} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And all scripted DALI exchanges should be consumed without errors
    Given a cct 3000K target-state script for short address 0
    When I PUT JSON {"color_mode":"cct","color_temperature_kelvin":3000} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-041
  Scenario: Direct target state does not create an operation entry
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the operations list should contain exactly 1 operation

  @id:PD-042
  Scenario: Direct target state rejects runtime observation fields in the body
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I PUT JSON {"status":{"raw":0}} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"
    And the DALI mock transport should have received 0 forward frame
    When I PUT JSON {"waf":{"r":1,"g":2,"b":3}} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"
    And the DALI mock transport should have received 0 forward frame

  @id:PD-157
  Scenario: Colour-only target state preserves the stored runtime level
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the physical device 0 state level should eventually be 180
    Given a cct 3000K target-state script for short address 0
    When I PUT JSON {"color_mode":"cct","color_temperature_kelvin":3000} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And all scripted DALI exchanges should be consumed without errors
    And the physical device 0 state level should eventually be 180

  @id:PD-043
  @stage-R3
  Scenario: Direct target state rejects an unsupported color capability
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I PUT JSON {"color_mode":"rgb","rgb":{"r":255,"g":0,"b":0}} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 422
    And the JSON error should be "unsupported_capability"
    And the DALI mock transport should have received 0 forward frame

  @id:PD-037
  @stage-R6
  Scenario: color_mode_override unblocks a target state the scan evidence rejected
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"color_mode_override":"rgb"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    Given an rgb target-state script for short address 0
    When I PUT JSON {"color_mode":"rgb","rgb":{"r":255,"g":0,"b":0}} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-250
  @stage-R3
  Scenario: A mid-tone colour reaches the wire as linear dim levels
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"color_mode_override":"rgb"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    Given a mid-tone rgb target-state script for short address 0
    When I PUT JSON {"color_mode":"rgb","rgb":{"r":255,"g":180,"b":90}} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-179
  @stage-R3
  Scenario: A six-channel colour is refused on a fixture without the channels
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PUT JSON {"color_mode":"rgbwaf","rgbwaf":{"r":254,"g":0,"b":0,"w":0,"a":0,"f":0}} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 422
    And the JSON error should be "unsupported_capability"

  @id:PD-180
  @stage-R3
  Scenario: A partial six-channel colour is refused as a bad body, not a bad fixture
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    When I PUT JSON {"rgbwaf":{"r":254,"g":0,"b":0}} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
