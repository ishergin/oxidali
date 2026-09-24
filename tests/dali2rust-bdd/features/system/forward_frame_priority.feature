@stage-X1
Feature: Forward-frame priority — what we announce on the wire

  @id:SYS-235
  Scenario: An operator setpoint announces itself as user-instigated switching
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I PUT JSON {"power":"on","level":190} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the first forward frame should be sent at priority 2

  @id:SYS-236
  Scenario: A configuration write never claims the operator's priority
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 100
    When I POST JSON {"fade_time_ms":4000} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And no forward frame should be sent at priority 2

  @id:SYS-237
  Scenario: A scheduled setpoint announces itself as an automatic action
    Given the controller clock reads 420 minutes past midnight
    And HCL schedule "morning" exists
    Then the scheduler should drive group 1 to level 80
    And the first forward frame should be sent at priority 4
    And no forward frame should be sent at priority 2, 3 or 5

  @id:SYS-238
  Scenario: A poller read announces itself as a periodic query
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And the DALI mock transport trace is cleared
    And the DALI mock answers every query with 128
    When the poller is enabled with a 200 ms interval
    And the poller has completed a read
    Then the first forward frame should be sent at priority 5
    And no forward frame should be sent at priority 2, 3 or 4

  @id:SYS-239
  Scenario: Only the first frame of a unit carries its purpose
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 100
    When I POST JSON {"fade_time_ms":4000} to "/api/v1/adapters/0/physical-devices/0/write-attributes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the first forward frame should not be sent at priority 1
    And every forward frame after the first should be sent at priority 1

  @id:SYS-240
  Scenario: Two independent commands each open their own transaction
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I PUT JSON {"power":"on","level":190} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    When I PUT JSON {"power":"on","level":120} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And no forward frame should be sent at priority 1
