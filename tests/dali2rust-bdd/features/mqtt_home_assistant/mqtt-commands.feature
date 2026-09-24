@stage-I4
Feature: MQTT/HA — commands from Home Assistant reach the wire
  As an operator with a light switch in Home Assistant
  I want my command to become a semantic DALI command
  So that the bridge drives gear through the same path everything else does

  @id:MQTT-003
  Scenario: A brightness command on a lamp topic drives its bound gear
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/0/binding"
    Then the response status should be 200
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a successful target-state script for level 180 on short address 0
    When Home Assistant publishes {"state":"ON","brightness":180} on "dali/ctl1/a0/vl/0/set"
    Then the virtual lamp 0 level on adapter 0 should eventually be 180

  @id:MQTT-006
  Scenario: A command for another controller is counted, not obeyed
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When Home Assistant publishes {"state":"ON","brightness":180} on "dali/other/a0/vl/0/set"
    Then the MQTT unroutable-command count should eventually be at least 1

  @id:MQTT-004
  Scenario: A command naming no known entity is refused rather than misrouted
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When Home Assistant publishes {"state":"ON"} on "dali/ctl1/a0/vl/63/set"
    Then the MQTT unroutable-command count should eventually be at least 1

  @id:MQTT-019
  Scenario: Picking a scene recalls it by broadcast and the select tracks the active scene
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/0/binding"
    Then the response status should be 200
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a broadcast scene recall script for scene 3
    When Home Assistant publishes Scene 3 on "dali/ctl1/a0/scene_select/set"
    Then the scripted DALI exchanges should eventually be consumed
    Given a cct 3000K target-state script for short address 0
    When I PUT JSON {"color_mode":"cct","color_temperature_kelvin":3000} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And MQTT should have a retained "Scene 3" on "dali/ctl1/a0/scene_select/state"
    Given a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And MQTT should have a retained "None" on "dali/ctl1/a0/scene_select/state"
