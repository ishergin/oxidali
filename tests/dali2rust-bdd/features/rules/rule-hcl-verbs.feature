@stage-I10
Feature: Rules and HCL: a rule enables, disables, holds and resumes a schedule

  A rule names a schedule by its id and switches it through the registry, the
  single writer of schedules; the scheduler follows the stored bit. Two lamps
  in two groups of one schedule make the negative sound: what the rule names
  is suspended or resumed, and the other group keeps its state.

  Background:
    Given the controller clock reads 600 minutes past midnight
    And adapter 0 has discovered physical devices 0 and 1
    And physical device 0 reports membership of group 7 read from the gear
    And physical device 1 reports membership of group 8 read from the gear
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I PUT JSON {"physical_short_address":1} to "/api/v1/adapters/0/virtual-lamps/2/binding"
    Then the response status should be 200
    When I POST JSON {"schedule_id":"house","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[7,8]}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"absolute","level":80,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the scheduler should drive group 7 to level 80
    And the scheduler should drive group 8 to level 80

  @id:RULE-060
  Scenario: A rule disables a schedule by its id, the scheduler goes quiet, and enabling it drives again
    When I PUT JSON {"base_revision":0,"source":"rule \"off\" {\n  when http trigger\n  do hcl.disable(\"house\")\n}\nrule \"on\" {\n  when http trigger\n  do hcl.enable(\"house\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I POST JSON {} to "/api/v1/rules/off/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "house" is eventually disabled
    Given the DALI mock transport trace is cleared
    When the scheduler has run 3 more ticks
    Then no DALI frames should have reached the bus
    When I POST JSON {} to "/api/v1/rules/on/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "house" is eventually enabled
    And the scheduler should drive group 7 to level 80
    And the scheduler should drive group 8 to level 80
    And the rule "off" should have the last outcome "ok"

  @id:RULE-061
  Scenario: A rule holds one group of a schedule without touching the light, and the hold wakes the override trigger
    When I PUT JSON {"base_revision":0,"source":"rule \"hold\" {\n  when http trigger\n  do hcl.hold(group(7))\n}\nrule \"pulse\" {\n  when every 1s\n  do log(\"p\")\n}\nrule \"watch-7\" {\n  when hcl override starts for group(7)\n  do log(\"7\")\n}\nrule \"watch-8\" {\n  when hcl override starts for group(8)\n  do log(\"8\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the rule "pulse" eventually has fired at least 1 time
    Given the DALI mock transport trace is cleared
    When I POST JSON {} to "/api/v1/rules/hold/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "house" eventually reports group 7 suspended
    And HCL schedule "house" should not report group 8 suspended
    And the rule "watch-7" eventually has fired 1 time
    And the rule "watch-8" should have fired 0 times
    And the rule "hold" should have the last outcome "ok"
    And no DALI frames should have reached the bus

  @id:RULE-062
  Scenario: A group a rule held is driven again once a rule resumes it, and only that group
    When I PUT JSON {"base_revision":0,"source":"rule \"hold\" {\n  when http trigger\n  do hcl.hold(group(7))\n}\nrule \"resume\" {\n  when http trigger\n  do hcl.resume(group(7))\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I POST JSON {} to "/api/v1/rules/hold/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "house" eventually reports group 7 suspended
    Given the DALI mock transport trace is cleared
    When I POST JSON {} to "/api/v1/rules/resume/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "house" eventually reports itself running
    And the scheduler should drive group 7 to level 80
    When the scheduler has run 3 more ticks
    Then exactly 1 arc power level should have been driven

  @id:RULE-063
  Scenario: A rule holds a lamp and the schedule stands down on the group the gear reported the lamp in
    When I PUT JSON {"base_revision":0,"source":"rule \"hold\" {\n  when http trigger\n  do hcl.hold(lamp(2))\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I POST JSON {} to "/api/v1/rules/hold/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "house" eventually reports group 8 suspended
    And HCL schedule "house" should not report group 7 suspended
