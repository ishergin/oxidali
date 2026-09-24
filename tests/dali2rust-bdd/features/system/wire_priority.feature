@stage-X1
Feature: Wire priority — operator work never waits behind attended work

  @id:SYS-230
  Scenario: An operator setpoint does not wait out an attribute read
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 5
    And the DALI transport parks the next exchange for 500 milliseconds
    When I start a full attribute read for adapter 0 physical device 0
    Then the response status should be 202
    When I PUT JSON {"power":"on","level":190} to "/api/v1/adapters/0/physical-devices/0/target-state" in background
    And I drain the background request
    Then the response status should be 200
    And the operator setpoint for level 190 on short address 0 should reach the wire within 2 frames

  @id:SYS-231
  Scenario: A preempted read reports preempted and commits nothing
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 5
    And the DALI transport parks the next exchange for 500 milliseconds
    When I start a full attribute read for adapter 0 physical device 0
    Then the response status should be 202
    When I PUT JSON {"power":"on","level":190} to "/api/v1/adapters/0/physical-devices/0/target-state" in background
    Then the last operation eventually fails
    And the operation error code should be "preempted"
    And the operation attribute-read outcomes should show "scenes" as "not_attempted"
    When I drain the background request
    Then the response status should be 200

  @id:SYS-232
  Scenario: An attended write does not preempt an attended read
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 5
    And the DALI transport parks the next exchange for 500 milliseconds
    When I start a full attribute read for adapter 0 physical device 0
    Then the response status should be 202
    When I POST JSON {"fade_time_ms":500} to "/api/v1/adapters/0/physical-devices/0/write-attributes" in background
    Then the last operation eventually succeeds
    And the operation attribute-read outcomes should show "scenes" as "success"
    When I drain the background request
    Then the response status should be 202

  @id:SYS-233
  Scenario: write-attributes yields between attributes and keeps what it confirmed
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 100
    And the DALI transport parks the next exchange for 500 milliseconds
    When I POST JSON {"fade_time_ms":4000,"power_on_level":100} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    When I PUT JSON {"power":"on","level":190} to "/api/v1/adapters/0/physical-devices/0/target-state" in background
    Then the last operation eventually fails
    And the operation error code should be "preempted"
    And physical device 0 eventually exposes fade_time_ms 4000
    When I drain the background request
    Then the response status should be 200

  @id:SYS-234
  Scenario: Discovery holds its INITIALISE session against an operator setpoint
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden control-gear discovery script for short address 0
    And the DALI transport parks the next exchange for 500 milliseconds
    When I start a discovery run for adapter 0
    Then the response status should be 202
    When I PUT JSON {"power":"on","level":190} to "/api/v1/adapters/0/physical-devices/0/target-state" in background
    Then the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors
    When I drain the background request
    Then the operator setpoint for level 190 on short address 0 should eventually reach the wire

