@stage-I10
Feature: Rules and HCL: a rule may move a scheduled light without standing the schedule down

  A light action from a rule suspends the schedule for its target, as any
  manual command does, unless the rule says `hold_hcl false`: then its commit
  carries "not an override" from the command to the HCL scheduler, and the
  schedule keeps driving the target. Two lamps in two groups of one schedule
  make the negative sound: the scheduler takes commits in order, so once the
  second rule's lamp is reported suspended, the first rule's commit has been
  judged. The `pulse` rule proves the engine has read the override state once
  before anything moves, so an edge cannot hide in the first reading.

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

  @id:RULE-037
  Scenario: A rule that says hold_hcl false moves a scheduled lamp and the schedule keeps running
    When I PUT JSON {"base_revision":0,"source":"rule \"keep\" hold_hcl false {\n  when http trigger\n  do lamp(1).level(100)\n}\nrule \"take\" {\n  when http trigger\n  do lamp(2).level(100)\n}\nrule \"retake\" {\n  when http trigger\n  do lamp(1).level(120)\n}\nrule \"pulse\" {\n  when every 1s\n  do log(\"p\")\n}\nrule \"watch-7\" {\n  when hcl override starts for group(7)\n  do log(\"7\")\n}\nrule \"watch-8\" {\n  when hcl override starts for group(8)\n  do log(\"8\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the rule "pulse" eventually has fired at least 1 time
    When I POST JSON {} to "/api/v1/rules/keep/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I POST JSON {} to "/api/v1/rules/take/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "house" eventually reports group 8 suspended
    And HCL schedule "house" should not report group 7 suspended
    And the rule "watch-8" eventually has fired 1 time
    And the rule "watch-7" should have fired 0 times
    When I POST JSON {} to "/api/v1/rules/retake/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "house" eventually reports group 7 suspended
    And the rule "watch-7" eventually has fired 1 time

  @id:RULE-038
  Scenario: A scene recall from a rule that says hold_hcl false leaves the schedule running too
    Given adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And adapter 0 scene 3 desired row for virtual lamp 2 has level 100
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    And adapter 0 scene 3 write for short 1 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    When I PUT JSON {"base_revision":0,"source":"rule \"keep\" hold_hcl false {\n  when http trigger\n  do scene(3).recall(lamp(1))\n}\nrule \"take\" {\n  when http trigger\n  do scene(3).recall(lamp(2))\n}\nrule \"pulse\" {\n  when every 1s\n  do log(\"p\")\n}\nrule \"watch-7\" {\n  when hcl override starts for group(7)\n  do log(\"7\")\n}\nrule \"watch-8\" {\n  when hcl override starts for group(8)\n  do log(\"8\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the rule "pulse" eventually has fired at least 1 time
    When I POST JSON {} to "/api/v1/rules/keep/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I POST JSON {} to "/api/v1/rules/take/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "house" eventually reports group 8 suspended
    And HCL schedule "house" should not report group 7 suspended
    And the rule "watch-8" eventually has fired 1 time
    And the rule "watch-7" should have fired 0 times
    And the virtual lamp 1 runtime level should eventually be 100
