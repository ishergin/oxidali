@stage-I10
Feature: Rules and Part 303: a rule tells an occupancy sensor to cancel its hold or to catch movement

  `input(d,i).cancel_hold()` and `.catch_movement()` are Part 303 instance
  commands: one 24-bit frame each on the instance's own device and instance
  address, never the 16-bit gear pair. Only an occupancy-sensor instance takes
  them; a rule that names another instance fails before the wire.

  Background:
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "3,1"
    When input devices are scanned on adapter 0 and the scan succeeds
    When I PUT JSON {"base_revision":0,"source":"rule \"leave\" {\n  when http trigger\n  do input(dev=0, inst=0).cancel_hold()\n     input(dev=0, inst=0).catch_movement()\n}\nrule \"wrong\" {\n  when http trigger\n  do input(dev=0, inst=1).cancel_hold()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When the mock transport 24-bit trace is cleared

  @id:RULE-065
  Scenario: Cancel-hold and catch-movement go out as one 24-bit instance frame each, in the rule's order
    When I POST JSON {} to "/api/v1/rules/leave/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the mock transport 24-bit trace should eventually be exactly "01 00 24, 01 00 20"
    And the first 24-bit forward frame should be sent at priority 2
    And 24-bit forward frame 2 should be sent at priority 2
    And the DALI mock transport should have received 0 forward frame
    And the rule "leave" should have the last outcome "ok"

  @id:RULE-066
  Scenario: A rule that asks a push button to cancel a hold reaches no wire and fails
    When I POST JSON {} to "/api/v1/rules/wrong/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the rule "wrong" eventually has fired 1 time
    And the rule "wrong" should have the last outcome "failed"
    When I POST JSON {} to "/api/v1/rules/leave/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the mock transport 24-bit trace should eventually be exactly "01 00 24, 01 00 20"
