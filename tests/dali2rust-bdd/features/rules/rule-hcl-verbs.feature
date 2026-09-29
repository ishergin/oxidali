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
