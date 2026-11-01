@stage-I10
Feature: Rules dim a held button's light as an IEC command iteration

  `.dim_up()` / `.dim_down()` send one UP or DOWN frame per firing (IEC 62386-102
  §11.3.3, §11.3.4). Fired on every `long_press_repeat`, the frames form a command
  iteration (101 §9.4, 102 §9.8.2): the gear fades at its own fade rate for as long
  as the button is held, a group or broadcast as one frame. What level the gear
  reached is not computable, so once the iteration has gone quiet the controller
  reads each lamp the frames reached back from the gear, whether or not the poller
  is on. The old `.dim_hold(+N)` / `.dim_hold(-N)` keeps compiling as the same verb.

  @id:RULE-092
  Scenario: A held push button dims its lamp with one UP frame per repeat, then reads the level back
    Given the mock bus answers 24-bit query "0B FE 35" with "01"
    And the mock bus answers 24-bit query "0B 00 80" with "01"
    When input devices are scanned on adapter 0 and the scan succeeds
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PUT JSON {"base_revision":0,"source":"rule \"brighter\" {\n  when input(dev=5, inst=0) is long_press_repeat\n  do lamp(1).dim_up()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 128
    When a 24-bit input event frame 0A 80 0B is observed on the bus
    And a 24-bit input event frame 0A 80 0B is observed on the bus
    Then the DALI mock transport forward trace should eventually be exactly "0101, 0101, 0191, 0190, 01A0, 01C2, 01C3, 01C4"
    And the first forward frame should be sent at priority 2
    And the rule "brighter" should have fired 2 times
    And virtual lamp 1 on adapter 0 should eventually report level 128 from value_source "poller"

  @id:RULE-093
  Scenario: A group and broadcast dim is one DOWN or UP frame on the group or broadcast address
    When I PUT JSON {"base_revision":0,"source":"rule \"both\" {\n  when http trigger\n  do group(7).dim_down()\n     broadcast.dim_up()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {} to "/api/v1/rules/both/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the DALI mock transport forward trace should eventually be exactly "8F02, FF01"
    And the rule "both" should have the last outcome "ok"

  @id:RULE-094
  Scenario: The old dim_hold spelling still compiles and dims by its sign
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PUT JSON {"base_revision":0,"source":"rule \"darker\" {\n  when http trigger\n  do lamp(1).dim_hold(-60)\n}\nrule \"lighter\" {\n  when http trigger\n  do lamp(1).dim_hold(+60)\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 128
    When I POST JSON {} to "/api/v1/rules/darker/run"
    Then the response status should be 202
    And the DALI mock transport forward trace should eventually be exactly "0102, 0191, 0190, 01A0, 01C2, 01C3, 01C4"
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 128
    When I POST JSON {} to "/api/v1/rules/lighter/run"
    Then the response status should be 202
    And the DALI mock transport forward trace should eventually be exactly "0101, 0191, 0190, 01A0, 01C2, 01C3, 01C4"

  @id:RULE-095
  Scenario: A group dim reads back each lamp the group reaches
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    When I PUT JSON {"base_revision":0,"source":"rule \"hall\" {\n  when http trigger\n  do group(7).dim_down()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    And the DALI mock answers every query with 128
    When I POST JSON {} to "/api/v1/rules/hall/run"
    Then the response status should be 202
    And the DALI mock transport forward trace should eventually be exactly "8F02, 0191, 0190, 01A0, 01C2, 01C3, 01C4"
    And virtual lamp 1 on adapter 0 should eventually report level 128 from value_source "poller"
