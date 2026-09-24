@stage-I4
Feature: MQTT/HA — groups and the scene select
  As a controller with hardware groups and programmed scenes
  I want them published as entities of their own
  So that one broadcast frame does what sixteen would otherwise

  @id:MQTT-002
  Scenario: A group with members is published as a light, an empty one is not
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the MQTT payload on "homeassistant/light/ctl1/a0_group_7/config" should have string field "unique_id" = "ctl1_a0_group_7"
    And the MQTT payload on "homeassistant/light/ctl1/a0_group_7/config" should have string field "availability_topic" = "dali/ctl1/availability"
    And the MQTT payload on "homeassistant/light/ctl1/a0_group_7/config" should have string field "name" = "Group 7"
    And MQTT should have exactly 0 publishes on "homeassistant/light/ctl1/a0_group_0/config"

  @id:MQTT-001
  Scenario: The scene select is published with a placeholder before any recall
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/0/binding"
    Then the response status should be 200
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the MQTT payload on "homeassistant/select/ctl1/a0_scene_select/config" should have string field "unique_id" = "ctl1_a0_scene_select"
    And MQTT should have a retained "None" on "dali/ctl1/a0/scene_select/state"

  @id:MQTT-014
  Scenario: A republish retracts every entity the operator did not expose
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When I POST JSON {} to "/api/v1/settings/home-assistant/discovery-publish"
    Then the response status should be 202
    And the last operation eventually succeeds
    And MQTT should have a retained "" on "homeassistant/light/ctl1/a0_vl_63/config"
    And MQTT should have a retained "" on "homeassistant/light/ctl1/a0_group_3/config"

  @id:MQTT-024
  Scenario: The announced device carries the version the controller was composed with
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/0/binding"
    Then the response status should be 200
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_0/config" should have string field "device.sw_version" = "0.1.0-test"
