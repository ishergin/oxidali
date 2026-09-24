@stage-R1
Feature: A disabled adapter refuses work, and says so before its TTL

  @id:ADP-021
  Scenario: Write-attributes on a disabled adapter fails the operation instead of timing out
    When I PATCH JSON {"enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be false
    When I POST JSON {"fade_time_ms":500} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "conflict"
    And the operation error message should be "adapter_disabled"

  @id:ADP-022
  Scenario: Identify on a disabled adapter fails the operation
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    When I POST JSON {"short_address":0} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "conflict"
    And the operation error message should be "adapter_disabled"

  @id:ADP-023
  Scenario: An address change on a disabled adapter never reaches the wire
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    And the DALI mock transport frame log should be cleared
    When I POST JSON {"short_address":0,"new_short_address":7} to "/api/v1/adapters/0/commissioning/address-changes"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "conflict"
    And the operation error message should be "adapter_disabled"
    And the DALI mock transport should have received 0 forward frame

  @id:ADP-024
  Scenario: An expert commissioning step on a disabled adapter is refused before the wire
    When I PATCH JSON {"enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    When I POST JSON {} to "/api/v1/adapters/0/commissioning/steps/terminate"
    Then the response status should be 200
    And the JSON boolean field "success" should be false
    And the JSON field "error_code" should be "conflict"
    And the JSON field "message" should be "adapter_disabled"
    And the DALI mock transport should have received 0 forward frame
