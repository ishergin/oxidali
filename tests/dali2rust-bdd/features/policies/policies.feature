@stage-X1
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
