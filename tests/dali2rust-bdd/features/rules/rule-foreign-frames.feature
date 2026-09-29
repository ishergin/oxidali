@stage-I10
Feature: Rules and foreign frames: a frame moves only the lamps it reaches

  A frame addressed 0xFC or 0xFD reaches only gear that has no short
  address, and no record describes such gear: the translator decodes it and
  publishes nothing, so no lamp moves and no state rule wakes. The addressed
  frame after it is the positive control: it is translated, projected and
  committed after the unaddressed one, so once its rule has fired the verdict
  on the frame before it is in.

  @id:RULE-039
  Scenario: An OFF to gear without a short address switches no lamp off
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PUT JSON {"base_revision":0,"source":"rule \"dark\" cooldown 0ms {\n  when lamp(1) turns off\n  do log(\"d\")\n}\nrule \"bright\" cooldown 0ms {\n  when lamp(1).level crosses above 100\n  do log(\"b\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When a foreign DAPC frame for short address 0 level 90 is observed on the bus
    Then the virtual lamp 1 runtime level should eventually be 90
    When a foreign unaddressed OFF is observed on the bus
    And a foreign DAPC frame for short address 0 level 120 is observed on the bus
    Then the rule "bright" eventually has fired 1 time
    And the rule "dark" should have fired 0 times
