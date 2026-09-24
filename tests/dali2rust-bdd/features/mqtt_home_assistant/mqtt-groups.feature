@stage-I4
Feature: MQTT/HA — group commands and the group tile's meaning
  As an operator with lamps shared between groups
  I want a group tile to light only for the group that was actually commanded
  So that switching on group 1 does not paint every group containing its lamps

  @id:MQTT-017
  Scenario: A group command drives one group-addressed frame and its members' state
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a group direct-arc script for level 128 on group 7
    When Home Assistant publishes {"state":"ON","brightness":128} on "dali/ctl1/a0/group/7/set"
    Then the virtual lamp 1 level on adapter 0 should eventually be 128
    And the scripted DALI exchanges should eventually be consumed
    And the MQTT payload on "dali/ctl1/a0/group/7/state" should have string field "state" = "ON"

  @id:MQTT-018
  Scenario: A tile lights only for the commanded group, not for every group sharing a lamp
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in groups 1 and 2
    And adapter 0 group adds for short 0 groups 1 and 2 are scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And MQTT should have exactly 1 publish on "dali/ctl1/a0/group/1/state"
    And the MQTT payload on "dali/ctl1/a0/group/1/state" should have string field "state" = "OFF"
    And the MQTT payload on "dali/ctl1/a0/group/2/state" should have string field "state" = "OFF"
    Given a group direct-arc script for level 128 on group 1
    When Home Assistant publishes {"state":"ON","brightness":128} on "dali/ctl1/a0/group/1/set"
    Then MQTT should have exactly 2 publishes on "dali/ctl1/a0/group/1/state"
    And the MQTT payload on "dali/ctl1/a0/group/1/state" should have string field "state" = "ON"
    And the MQTT payload on "dali/ctl1/a0/group/2/state" should have string field "state" = "OFF"
    Given a group off script for group 1
    When Home Assistant publishes {"state":"OFF"} on "dali/ctl1/a0/group/1/set"
    Then MQTT should have exactly 3 publishes on "dali/ctl1/a0/group/1/state"
    And the MQTT payload on "dali/ctl1/a0/group/1/state" should have string field "state" = "OFF"
