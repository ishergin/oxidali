@stage-I3
Feature: HCL daily override visibility and reset

  When somebody drives a light a schedule owns, the schedule stands down for
  the rest of the local day. That flag lives in the scheduler thread, so from
  outside a suspended schedule looks exactly like a running one — enabled, with
  a next point, and silent. This sub-resource is what tells them apart, and the
  only way to resume before midnight.

  Raising a flag needs an anchored clock and a tick that actually published, so
  the suspended-state behaviour is proven where it can be driven: the scheduler
  crate tests. What a client can observe here is the contract.

  @id:HCL-072
  Scenario: A schedule that has not been overridden reports itself running
    Given HCL schedule "morning" exists
    When I send a GET request to "/api/v1/hcl-schedules/morning/override"
    Then the response status should be 200
    And the JSON boolean field "suspended" should be false
    And the HCL override target list should be empty
    And the JSON field "since_local_minutes" should be absent

  @id:HCL-073
  Scenario: The override of an unknown schedule is not found
    When I send a GET request to "/api/v1/hcl-schedules/never-existed/override"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:HCL-074
  Scenario: Resetting the override of an unknown schedule is not found
    When I send a DELETE request to "/api/v1/hcl-schedules/never-existed/override"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:HCL-075
  Scenario: Resetting a schedule that holds no flag still succeeds
    Given HCL schedule "morning" exists
    When I send a DELETE request to "/api/v1/hcl-schedules/morning/override"
    Then the response status should be 204
    When I send a GET request to "/api/v1/hcl-schedules/morning/override"
    Then the response status should be 200
    And the JSON boolean field "suspended" should be false

  @id:HCL-076
  Scenario: The override is a sub-resource, not a schedule field
    Given HCL schedule "morning" exists
    When I send a GET request to "/api/v1/hcl-schedules/morning"
    Then the response status should be 200
    And the JSON field "suspended" should be absent
    And the JSON field "override" should be absent

  @id:HCL-077
  Scenario: A manual level command leaves a colour-only schedule running
    Given the controller clock reads 600 minutes past midnight
    And a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a cct 3000K target-state script for short address 0
    When I PUT JSON {"color_mode":"cct","color_temperature_kelvin":3000} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And physical device 0 should hold a stored colour
    When the scheduler has run 2 more ticks
    Given the DALI mock transport trace is cleared
    When I POST JSON {"schedule_id":"colour-only","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"broadcast"}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"none","level":null,"color_temperature_kelvin":4000}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When the scheduler has run 3 more ticks
    Then some DALI frames should have reached the bus
    Given a successful target-state script for level 128 on short address 0
    When I PUT JSON {"power":"on","level":128} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    When the scheduler has run 3 more ticks
    And I send a GET request to "/api/v1/hcl-schedules/colour-only/override"
    Then the response status should be 200
    And the JSON boolean field "suspended" should be false

  @id:HCL-078
  Scenario: The same command still stands down a schedule that drives the level
    Given the controller clock reads 600 minutes past midnight
    And a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I POST JSON {"schedule_id":"level-driving","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"broadcast"}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"absolute","level":80,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the scheduler should drive broadcast to level 80
    Given a successful target-state script for level 128 on short address 0
    When I PUT JSON {"power":"on","level":128} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    Then HCL schedule "level-driving" eventually reports itself suspended
