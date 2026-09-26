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

  @id:ADP-025
  Scenario: Part 103 commissioning on a disabled adapter never reaches the wire
    When I PATCH JSON {"enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    When I POST JSON {} to "/api/v1/adapters/0/input-devices/commission"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "conflict"
    And the operation error message should be "adapter_disabled"
    And the mock transport should have sent no 24-bit frames
    And the DALI mock transport should have received 0 forward frame

  @id:ADP-026
  Scenario: Scene programming on a disabled adapter never reaches the wire
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    When I PATCH JSON {"enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    And the DALI mock transport frame log should be cleared
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "conflict"
    And the operation error message should be "adapter_disabled"
    And the DALI mock transport should have received 0 forward frame

  @id:ADP-027
  Scenario: A synchronous command on a disabled adapter names the cause of its 409
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    When I PATCH JSON {"enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    And the DALI mock transport frame log should be cleared
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 409
    And the JSON field "error" should be "conflict"
    And the JSON field "message" should be "adapter_disabled"
    And the DALI mock transport should have received 0 forward frame
