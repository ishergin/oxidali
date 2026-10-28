@stage-R17
Feature: Policies — what a luminaire does without a controller

  `ADR-018` D7. `systemFailureLevel` and `powerOnLevel` are the two variables
  that decide what the building looks like when the controller is gone, which
  is precisely the case redundancy exists for — and they are written to the
  gear, once, rather than held here and applied on demand.

  Unmanaged is the default and is not the same as zero: this controller writes
  the variable on no device at all until somebody chooses a level.

  @id:POLICY-001
  Scenario: Both levels start unmanaged and an apply would write nothing
    When I send a GET request to "/api/v1/policies"
    Then the response status should be 200
    And the JSON pointer "/system_failure_level" should be null
    And the JSON pointer "/power_on_level" should be null
    And the JSON boolean field "apply_on_discovery" should be false
    And the JSON boolean field "manages_anything" should be false
    And the JSON field "exceptions" should equal []

  @id:POLICY-002
  Scenario: Choosing a level makes the policy manage something
    When I PATCH JSON {"system_failure_level":254,"power_on_level":100} to "/api/v1/policies"
    Then the response status should be 200
    And the JSON numeric field "system_failure_level" should be 254
    And the JSON numeric field "power_on_level" should be 100
    And the JSON boolean field "manages_anything" should be true
    When I send a GET request to "/api/v1/policies"
    Then the JSON numeric field "system_failure_level" should be 254

  @id:POLICY-003
  Scenario: null returns a level to unmanaged
    When I PATCH JSON {"system_failure_level":100} to "/api/v1/policies"
    Then the response status should be 200
    When I PATCH JSON {"system_failure_level":null} to "/api/v1/policies"
    Then the response status should be 200
    And the JSON pointer "/system_failure_level" should be null

  @id:POLICY-004
  Scenario: MASK is refused rather than passed through
    When I PATCH JSON {"power_on_level":255} to "/api/v1/policies"
    Then the response status should be 422
    And the JSON error should be "out_of_range"

  @id:POLICY-005
  Scenario: An unknown key is refused rather than ignored
    When I PATCH JSON {"nope":1} to "/api/v1/policies"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:POLICY-006
  Scenario: An apply over an empty policy is refused, not performed
    When I send a POST request to "/api/v1/policies/apply" with empty body
    Then the response status should be 409
    And the JSON error should be "nothing_managed"

  @id:POLICY-007
  Scenario: An apply reaches every known device, not only the first
    Given adapter 0 has discovered physical devices 0 and 1
    When I PATCH JSON {"system_failure_level":0,"power_on_level":0} to "/api/v1/policies"
    Then the response status should be 200
    Given a policy write script setting both levels to 0 on short addresses 0 and 1
    When I send a POST request to "/api/v1/policies/apply" with empty body
    Then the response status should be 202
    And the last operation eventually succeeds
    And physical device 1 eventually exposes write-confirmed common_102 power_on_level 0
    And all scripted DALI exchanges should be consumed without errors

  @id:POLICY-008
  Scenario: A finished discovery starts the policy apply the operator asked for
    When I PATCH JSON {"system_failure_level":0,"power_on_level":0,"apply_on_discovery":true} to "/api/v1/policies"
    Then the response status should be 200
    And the JSON boolean field "apply_on_discovery" should be true
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/operations"
    Then the response status should be 200
    And the operations list should contain a key starting with "policy-apply-0-"

  @id:POLICY-009
  Scenario: A discovery starts no policy apply when the toggle is off
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/operations"
    Then the response status should be 200
    And the operations list should contain no key starting with "policy-apply-"

  @id:POLICY-010
  Scenario: A level the gear refuses fails the apply as not confirmed, without waiting out the cell
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"power_on_level":200} to "/api/v1/policies"
    Then the response status should be 200
    Given a policy write of power-on level 200 that short address 0 answers with 150 on every attempt
    When I send a POST request to "/api/v1/policies/apply" with empty body
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "verify_failed"
    And the operation error message should be "policy_not_confirmed"
    And all scripted DALI exchanges should be consumed without errors

  @id:POLICY-011
  Scenario: A read-back the gear leaves unanswered fails the apply as unverified
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"power_on_level":200} to "/api/v1/policies"
    Then the response status should be 200
    Given a policy write of power-on level 200 that short address 0 leaves unanswered
    When I send a POST request to "/api/v1/policies/apply" with empty body
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "verify_unanswered"
    And the operation error message should be "verify_unanswered"
    And all scripted DALI exchanges should be consumed without errors

  @id:POLICY-012
  Scenario: The derived manages_anything is read-only, not unknown
    When I PATCH JSON {"manages_anything":true} to "/api/v1/policies"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

  @id:POLICY-013
  Scenario: Exceptions start empty and a PATCH stores the excepted devices in address order
    Given adapter 0 has discovered physical devices 0 and 1
    When I send a GET request to "/api/v1/policies"
    Then the JSON field "exceptions" should equal []
    When I PATCH JSON {"exceptions":[{"adapter_id":0,"short_address":1},{"adapter_id":0,"short_address":0}]} to "/api/v1/policies"
    Then the response status should be 200
    And the JSON field "exceptions" should equal [{"adapter_id":0,"short_address":0},{"adapter_id":0,"short_address":1}]
    When I send a GET request to "/api/v1/policies"
    Then the JSON field "exceptions" should equal [{"adapter_id":0,"short_address":0},{"adapter_id":0,"short_address":1}]

  @id:POLICY-014
  Scenario: An exception naming a device the registry does not hold is refused
    Given adapter 0 has discovered physical devices 0 and 1
    When I PATCH JSON {"exceptions":[{"adapter_id":0,"short_address":7}]} to "/api/v1/policies"
    Then the response status should be 422
    And the JSON error should be "unknown_device"
    When I send a GET request to "/api/v1/policies"
    Then the JSON field "exceptions" should equal []

  @id:POLICY-015
  Scenario: A device named twice is refused rather than stored once
    Given adapter 0 has discovered physical devices 0 and 1
    When I PATCH JSON {"exceptions":[{"adapter_id":0,"short_address":1},{"adapter_id":0,"short_address":1}]} to "/api/v1/policies"
    Then the response status should be 422
    And the JSON error should be "duplicate_exception"

  @id:POLICY-016
  Scenario: A list longer than the limit is refused before any device is looked up
    When I PATCH a policy exception list naming 33 devices
    Then the response status should be 422
    And the JSON error should be "too_many_exceptions"
    When I PATCH a policy exception list naming 32 devices
    Then the response status should be 422
    And the JSON error should be "unknown_device"

  @id:POLICY-017
  Scenario: An apply writes nothing to an excepted device and names it as skipped
    Given adapter 0 has discovered physical devices 0 and 1
    When I PATCH JSON {"system_failure_level":0,"power_on_level":0,"exceptions":[{"adapter_id":0,"short_address":1}]} to "/api/v1/policies"
    Then the response status should be 200
    Given a policy write script setting both levels to 0 on short address 0 only
    When I send a POST request to "/api/v1/policies/apply" with empty body
    Then the response status should be 202
    And the last operation eventually succeeds
    And the policy-apply operation result should list written short addresses [0] and skipped short addresses [1]
    And every skipped short address in the policy-apply operation result should give the reason "policy_exception"
    And the DALI mock transport should have stored the power-on level before the system-failure level on short address 0
    And the DALI mock transport should have sent no SET POWER ON LEVEL or SET SYSTEM FAILURE LEVEL to short address 1
    And all scripted DALI exchanges should be consumed without errors

  @id:POLICY-018
  Scenario: A discovery-started apply skips an excepted device as well
    Given adapter 0 has discovered physical devices 0 and 1
    When I PATCH JSON {"system_failure_level":0,"power_on_level":0,"apply_on_discovery":true,"exceptions":[{"adapter_id":0,"short_address":1}]} to "/api/v1/policies"
    Then the response status should be 200
    Given a discovery script for physical devices 0 and 1 followed by a policy write setting both levels to 0 on short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the policy apply started by the discovery eventually succeeds
    And the policy-apply operation result should list written short addresses [0] and skipped short addresses [1]
    And the DALI mock transport should have stored the power-on level before the system-failure level on short address 0
    And the DALI mock transport should have sent no SET POWER ON LEVEL or SET SYSTEM FAILURE LEVEL to short address 1
    And all scripted DALI exchanges should be consumed without errors

  @id:POLICY-019
  Scenario: A read that finds a managed level drifted flags the device, and a read that matches clears it
    Given adapter 0 has discovered physical devices 0 and 1
    When I PATCH JSON {"system_failure_level":254,"power_on_level":100} to "/api/v1/policies"
    Then the response status should be 200
    Given a common-102 attribute-read script where short address 0 holds power-on level 50 and system-failure level 254
    When I start an attribute read for adapter 0 physical device 0 with attribute group "common_102" only
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON pointer "/policy_mismatch" should be true
    Given a common-102 attribute-read script where short address 0 holds power-on level 100 and system-failure level 254
    When I start an attribute read for adapter 0 physical device 0 with attribute group "common_102" only
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON pointer "/policy_mismatch" should be false

  @id:POLICY-020
  Scenario: A confirmed write of the policy clears the drift flag a read raised
    Given adapter 0 has discovered physical devices 0 and 1
    When I PATCH JSON {"system_failure_level":0,"power_on_level":0} to "/api/v1/policies"
    Then the response status should be 200
    Given a common-102 attribute-read script where short address 0 holds power-on level 50 and system-failure level 0
    When I start an attribute read for adapter 0 physical device 0 with attribute group "common_102" only
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON pointer "/policy_mismatch" should be true
    Given a policy write script setting both levels to 0 on short addresses 0 and 1
    When I send a POST request to "/api/v1/policies/apply" with empty body
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON pointer "/policy_mismatch" should be false

  @id:POLICY-021
  Scenario: The drift flag is absent while the policy manages nothing and for an excepted device
    Given adapter 0 has discovered physical devices 0 and 1
    Given a common-102 attribute-read script where short address 0 holds power-on level 50 and system-failure level 254
    When I start an attribute read for adapter 0 physical device 0 with attribute group "common_102" only
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON pointer "/policy_mismatch" should be absent
    When I PATCH JSON {"power_on_level":100} to "/api/v1/policies"
    Then the response status should be 200
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON pointer "/policy_mismatch" should be true
    When I PATCH JSON {"exceptions":[{"adapter_id":0,"short_address":0}]} to "/api/v1/policies"
    Then the response status should be 200
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the JSON pointer "/policy_mismatch" should be absent

  @id:POLICY-022
  Scenario: A policy change announces on the WebSocket each device whose drift flag it moved
    Given adapter 0 has discovered physical devices 0 and 1
    Given a common-102 attribute-read script where short address 0 holds power-on level 50 and system-failure level 254
    When I start an attribute read for adapter 0 physical device 0 with attribute group "common_102" only
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "physical_devices"
    When I PATCH JSON {"power_on_level":100} to "/api/v1/policies"
    Then the response status should be 200
    And the WebSocket client should receive a "PhysicalDeviceChangedEvent" frame on channel "physical_devices" whose payload "short_address" is 0
