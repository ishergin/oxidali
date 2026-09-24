@stage-I3
Feature: HCL scheduler runtime

  Every scenario anchors the controller clock through the production
  `PUT /api/v1/time` surface, so what the scheduler acts on is real local time
  rather than a test hook. Schedules arrive through the REST CRUD, and the
  proof is what reaches the mock DALI transport.

  The bus-level rules the wire cannot show — coalescing keys, the 32-command
  tick cap, curve evaluation itself — are crate-proof in
  `dali2rust-hcl-runtime` (policy `SCN-064`). `HCL-070` (an unanchored clock
  drives nothing) is there too, and not by preference: `SystemWallClock`
  reports a plausible system clock as already anchored — right on a device
  whose RTC survived or whose SNTP landed before we looked, but it makes
  "no anchor" unreachable from a host stack. Only a stub clock can state
  that premise, so `an_unanchored_clock_keeps_the_scheduler_silent` in
  `tests/scheduler_flow.rs` owns it.

  @id:HCL-050
  Scenario: A stepped point reaches the group it names
    Given the controller clock reads 420 minutes past midnight
    And HCL schedule "morning" exists
    Then the scheduler should drive group 1 to level 80

  @id:HCL-053
  Scenario: A held point is driven once, not every tick
    Given the controller clock reads 420 minutes past midnight
    And HCL schedule "morning" exists
    Then the scheduler should drive group 1 to level 80
    When the scheduler has run 3 more ticks
    Then exactly 2 arc power levels should have been driven

  @id:HCL-051
  Scenario: A disabled schedule drives nothing
    Given the controller clock reads 420 minutes past midnight
    When I POST JSON {"schedule_id":"off","enabled":false,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[1]}],"points":[{"time_ref":"absolute","offset_minutes":360,"level_mode":"absolute","level":80,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When the scheduler has run 3 more ticks
    Then no DALI frames should have reached the bus

  @id:HCL-064
  Scenario: A day the schedule does not cover drives nothing
    Given the controller clock reads 420 minutes past midnight
    When I POST JSON {"schedule_id":"weekend","enabled":true,"algorithm":"stepped","active_days":["sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[1]}],"points":[{"time_ref":"absolute","offset_minutes":360,"level_mode":"absolute","level":80,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When the scheduler has run 3 more ticks
    Then no DALI frames should have reached the bus

  @id:HCL-063
  Scenario: An absolute point drives both the level and the colour
    Given the controller clock reads 600 minutes past midnight
    When I POST JSON {"schedule_id":"noon","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[4]}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"absolute","level":200,"color_temperature_kelvin":4000}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the scheduler should drive group 4 to level 200

  @id:HCL-062
  Scenario: A last-active point recalls the gear's own level
    Given the controller clock reads 1200 minutes past midnight
    When I POST JSON {"schedule_id":"evening","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[6]}],"points":[{"time_ref":"absolute","offset_minutes":1140,"level_mode":"last_active","level":null,"color_temperature_kelvin":2200}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the scheduler should recall the last active level on group 6

  @id:HCL-060
  Scenario: A colour-only point never touches brightness
    Given the controller clock reads 600 minutes past midnight
    When I POST JSON {"schedule_id":"colour","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[2]}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"none","level":null,"color_temperature_kelvin":2700}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When the scheduler has run 3 more ticks
    Then some DALI frames should have reached the bus
    And no arc power level should have been driven

  @id:HCL-061
  Scenario: A point with neither level nor colour is a no-op
    Given the controller clock reads 600 minutes past midnight
    When I POST JSON {"schedule_id":"empty","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[2]}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"none","level":null,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When the scheduler has run 3 more ticks
    Then no DALI frames should have reached the bus

  @id:HCL-057
  Scenario: One schedule fans out to a group and to broadcast
    Given the controller clock reads 600 minutes past midnight
    When I POST JSON {"schedule_id":"house","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[7]},{"adapter_id":0,"scope":"broadcast"}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"absolute","level":120,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the scheduler should drive group 7 to level 120
    And the scheduler should drive broadcast to level 120

  @id:HCL-054
  Scenario: An interpolated curve drives the value between its points
    Given the controller clock reads 420 minutes past midnight
    When I POST JSON {"schedule_id":"ramp","enabled":true,"algorithm":"interpolated","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[8]}],"points":[{"time_ref":"absolute","offset_minutes":360,"level_mode":"absolute","level":80,"color_temperature_kelvin":null},{"time_ref":"absolute","offset_minutes":480,"level_mode":"absolute","level":200,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the scheduler should drive group 8 to level 140

  @id:HCL-056
  Scenario: A sunset point fires at the sun's time, not a fixed one
    Given the controller clock reads 1275 minutes past midnight
    When I POST JSON {"schedule_id":"dusk","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":{"latitude_deg":55.7558,"longitude_deg":37.6173},"targets":[{"adapter_id":0,"scope":"group","group_ids":[9]}],"points":[{"time_ref":"sunset","offset_minutes":-30,"level_mode":"absolute","level":60,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the scheduler should drive group 9 to level 60
