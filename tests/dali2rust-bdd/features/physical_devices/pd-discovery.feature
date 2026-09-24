@stage-R3
Feature: Adapter discovery runs HTTP contract

  @id:PD-100
  Scenario: POST discovery-runs accepts scan_known_short_addresses
    When I POST JSON {"mode":"scan_known_short_addresses"} to "/api/v1/adapters/0/discovery-runs"
    Then the response status should be 202
    And the JSON field "type" should be "discovery"
    And the JSON field "status" should be "accepted"
    And the JSON field "operation_id" should be non-empty

  @id:PD-101
  Scenario: POST discovery-runs rejects invalid mode
    When I POST JSON {"mode":"not_a_real_mode"} to "/api/v1/adapters/0/discovery-runs"
    Then the response status should be 422
    And the JSON error should be "invalid_enum"

  @id:PD-102
  Scenario: Discovery run verifies control-gear random address before publishing the physical device
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    And the discovery transport trace should match the golden control-gear identity flow
    And adapter 0 physical device 0 eventually exposes the discovered random address and DT8 identity
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-103
  Scenario: Discovery keeps earlier verified devices when a later identity verification times out
    Given a discovery script where short address 0 verifies before short address 1 times out
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually fails
    And adapter 0 physical devices eventually include only the verified device 0
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-104
  Scenario: A violating QueryShortAddress answer is recovered by re-arming, not by re-asking
    Given a discovery script where foreign-master activity corrupts the QueryShortAddress backward window
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    And the discovery transport trace should re-arm before re-asking QueryShortAddress
    And adapter 0 physical device 0 eventually exposes the discovered random address and DT8 identity
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-105
  Scenario: Discovery classifies a multi-DT MASK advertisement through QueryNextDeviceType enumeration
    Given a discovery script where QueryDeviceType returns MASK before QueryNextDeviceType enumerates DT6 and DT8
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    And the discovery transport trace should enumerate DT6 and DT8 after a MASK device-type advertisement
    And adapter 0 physical device 0 eventually exposes the discovered random address and DT8 identity
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-106
  Scenario: Refresh run keeps the verified random address of a known device
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes the discovered random address and DT8 identity
    Given a refresh detect-only script for short address 0
    When I start a refresh discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes the discovered random address and DT8 identity
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-107
  Scenario: Clean scan without answers keeps a previously verified device listed
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a discovery script where no control gear answers the presence sweep
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical devices eventually include only the verified device 0
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-189
  Scenario: A MASK advertisement stores every enumerated device type, not just the classification
    Given a discovery script where QueryDeviceType returns MASK before QueryNextDeviceType enumerates DT6 and DT8
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually declares device types 6, 8
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-190
  Scenario: A confirmed DT8 fallback probe joins the declared set
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    And adapter 0 physical device 0 eventually declares device types 0, 8
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-191
  Scenario: A device-type enumeration that never terminates reports no declared set at all
    Given a discovery script where the QueryNextDeviceType walk never reaches the 254 terminator
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually declares no known device types
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-194
  Scenario: A permanently violating QueryShortAddress reports several responders, not a wire fault
    Given a discovery script where two gear answer one search address for the whole verify
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error message should mention several responders
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-240
  Scenario: A scan of every short address reports its terminal outcome
    Given a discovery script for all 64 short addresses
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds within the full-segment budget
    And adapter 0 physical devices eventually include all 64 short addresses
    And all scripted DALI exchanges should be consumed without errors
