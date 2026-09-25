@stage-R2
Feature: Physical device write-attributes

  @id:PD-030
  Scenario: Write-attributes drives the DALI write sequence and tracks an operation
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a fade-time 500ms write-attributes script for short address 0
    When I POST JSON {"fade_time_ms":500} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the JSON field "type" should be "attribute_write"
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-034
  Scenario: Fade-time write then attribute read returns the canonical IEC milliseconds
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a fade-time 2000ms write-attributes script for short address 0
    When I POST JSON {"fade_time_ms":2000} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a common-102 attribute-read script with fade byte 0x47 for short address 0
    When I start an attribute read for adapter 0 physical device 0 with attribute group "common_102" only
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes fade_time_ms 2000
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-241
  Scenario: A fade-time write confirms the accepted code, not the requested milliseconds
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a fade-time 500ms write-attributes script for short address 0
    When I POST JSON {"fade_time_ms":500} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes fade_time_ms 700
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-242
  Scenario: A sub-350ms fade-time write never selects the extended-fade code
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a fade-time 100ms write-attributes script for short address 0
    When I POST JSON {"fade_time_ms":100} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes fade_time_ms 700
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-243
  Scenario: The top of the fade-time table survives a write round trip
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a fade-time 90500ms write-attributes script for short address 0
    When I POST JSON {"fade_time_ms":90500} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes fade_time_ms 90500
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-244
  Scenario: Write-attributes rejects a fade time past the end of the IEC table
    When I POST JSON {"fade_time_ms":90501} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frame

  @id:PD-158
  Scenario: Extended fade time write then extended read returns the canonical milliseconds
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an extended-fade-time 500ms write-attributes script for short address 0
    When I POST JSON {"extended_fade_time_ms":500} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an extended attribute-read script with fade byte 0x14 for short address 0
    When I start an attribute read for adapter 0 physical device 0 with attribute group "extended" only
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes extended fade_time_ms 500 as read back
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-038
  Scenario: Write-attributes answers 202 without waiting for the bus
    Given a bus with confirmation timeout of 500 milliseconds
    And the DALI transport blocks indefinitely
    When I POST JSON {"fade_time_ms":500} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response arrives within 250 milliseconds
    And the response status should be 202
    And the JSON field "type" should be "attribute_write"

  @id:PD-032
  Scenario: Write-attributes rejects discovered and runtime fields in the body
    When I POST JSON {"device_type_discovered":"dt6_led"} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"
    And the DALI mock transport should have received 0 forward frame

  @id:PD-033
  Scenario: Write-attributes rejects an out-of-range fade_rate
    When I POST JSON {"fade_rate":16} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And the DALI mock transport should have received 0 forward frame

  @id:PD-181
  Scenario: A Tc-limit write proves its DTR triple, sends the 242 pair and republishes the range
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a tc-limit write script for short address 0 storing coolest 200 and warmest 350
    When I POST JSON {"tc_coolest_mirek":200,"tc_warmest_mirek":350} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the JSON field "type" should be "attribute_write"
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes colour temperature range 2857 to 5000 kelvin
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-182
  Scenario: A Tc-limit pair with coolest above warmest never reaches the wire
    When I POST JSON {"tc_coolest_mirek":350,"tc_warmest_mirek":200} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And the DALI mock transport should have received 0 forward frame

  @id:PD-184
  Scenario: A min-level write proves its DTR triple and lands as a write-confirmed attribute
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a min-level 100 write-attributes script for short address 0
    When I POST JSON {"min_level":100} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the JSON field "type" should be "attribute_write"
    And the last operation eventually succeeds
    And physical device 0 eventually exposes write-confirmed common_102 min_level 100
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-185
  Scenario: A clamped min-level write confirms the accepted value, not the requested one
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a min-level write script for short address 0 where 220 is clamped to 200
    When I POST JSON {"min_level":220} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes write-confirmed common_102 min_level 200
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-186
  Scenario: Raising both bounds past the old roof re-drives min after max widens the interval
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a min-max write script for short address 0 raising 150 and 200 over old max 100
    When I POST JSON {"min_level":150,"max_level":200} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes write-confirmed common_102 min_level 150
    And physical device 0 eventually exposes write-confirmed common_102 max_level 200
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-187
  Scenario: A min level above the max level in one body never reaches the wire
    When I POST JSON {"min_level":200,"max_level":100} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And the DALI mock transport should have received 0 forward frame

  @id:PD-188
  Scenario: Lowering both bounds needs no extra triple
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a min-max write script for short address 0 lowering to 10 and 30
    When I POST JSON {"min_level":10,"max_level":30} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes write-confirmed common_102 min_level 10
    And physical device 0 eventually exposes write-confirmed common_102 max_level 30
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-183
  Scenario: A silently voided 242 pair fails the operation instead of confirming the old value
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a tc-limit write script where the pair never lands on short address 0
    When I POST JSON {"tc_coolest_mirek":200} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "operation_failed"
    And the operation error message should be "dt8_tc_limit_unconfirmed"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-252
  Scenario: A dimming-curve write pairs under its own prelude and reads back
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a dimming-curve 1 write-attributes script for short address 0
    When I POST JSON {"dimming_curve":1} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the JSON field "type" should be "attribute_write"
    And the last operation eventually succeeds
    And physical device 0 eventually exposes write-confirmed dt6_led dimming_curve 1
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-253
  Scenario: The logarithmic curve is a value, not an absence
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a dimming-curve 0 write-attributes script for short address 0
    When I POST JSON {"dimming_curve":0} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 0 eventually exposes write-confirmed dt6_led dimming_curve 0
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-254
  Scenario: A gear that ignores the curve leaves the attribute without write provenance
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a dimming-curve 1 write script for short address 0 that the gear ignores
    When I POST JSON {"dimming_curve":1} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-267
  Scenario: A fade-time write whose read-back goes unanswered is named unverified and confirms nothing
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a fade-time 500ms write script for short address 0 whose read-back goes unanswered
    When I POST JSON {"fade_time_ms":500} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "verify_unanswered"
    And physical device 0 common_102 fade_time_ms carries no write provenance
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-268
  Scenario: A dimming-curve write whose read-back goes unanswered is named unverified and confirms nothing
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a dimming-curve 1 write script for short address 0 whose read-back goes unanswered
    When I POST JSON {"dimming_curve":1} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "verify_unanswered"
    And physical device 0 dt6_led dimming_curve carries no write provenance
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-255
  Scenario: A reserved dimming-curve value is refused before it reaches the wire
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"dimming_curve":2} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And the DALI mock transport should have received 0 forward frame
