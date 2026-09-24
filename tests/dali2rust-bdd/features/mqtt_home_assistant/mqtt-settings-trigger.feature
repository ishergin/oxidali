@stage-I4
Feature: MQTT/HA — settings and capability changes reach the broker
  As an operator changing the bridge's configuration or a lamp's declared colour
  I want the broker's picture to follow without a manual republish
  So that Home Assistant never keeps serving a config the controller has outgrown

  @id:MQTT-009
  Scenario: A discovery republish reports how many entities it announced
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When I POST JSON {} to "/api/v1/settings/home-assistant/discovery-publish"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the last operation result should report at least 1 published entity
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_1/config" should have string field "unique_id" = "ctl1_a0_vl_1"

  @id:MQTT-010
  Scenario: Changing the broker endpoint restarts the session and re-announces
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    Then MQTT should have a retained "online" on "dali/ctl1/availability"
    When I PATCH JSON {"broker_host":"broker2.test"} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    And the MQTT broker should eventually observe 2 connects
    And MQTT should have exactly 3 publishes on "dali/ctl1/availability"
    And MQTT should have a retained "online" on "dali/ctl1/availability"

  @id:MQTT-008
  Scenario: A colour-mode override republishes the bound lamp's discovery config
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And MQTT should have exactly 1 publish on "homeassistant/light/ctl1/a0_vl_1/config"
    When I PATCH JSON {"color_mode_override":"rgb"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And MQTT should have exactly 2 publishes on "homeassistant/light/ctl1/a0_vl_1/config"
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_1/config" array field "supported_color_modes" should contain "rgb"
