@stage-I4
Feature: MQTT/HA — availability and per-commit state
  As a controller published into Home Assistant
  I want every entity to say whether I am alive and what it is doing
  So that a dashboard never shows a confident lie

  @id:MQTT-011
  Scenario: Enabling the bridge registers a will and announces itself first
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    Then the MQTT session should carry a retained last will on "dali/ctl1/availability"
    And MQTT should have a retained "online" on "dali/ctl1/availability"
    And the birth on "dali/ctl1/availability" should precede every discovery config

  @id:MQTT-005
  Scenario: A registry commit publishes lamp state exactly once
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
    And MQTT should have exactly 1 publish on "dali/ctl1/a0/vl/0/state"
    And the MQTT payload on "dali/ctl1/a0/vl/0/state" should have string field "state" = "ON"
    And the MQTT payload on "dali/ctl1/a0/vl/0/state" should have numeric field "brightness" = 180

  @id:MQTT-007
  Scenario: The discovery payload keys the entity on the lamp, not on its binding
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
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_0/config" should have string field "unique_id" = "ctl1_a0_vl_0"
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_0/config" should list availability topic "dali/ctl1/availability"
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_0/config" should list availability topic "dali/ctl1/a0/vl/0/availability"
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_0/config" should have string field "availability_mode" = "all"
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_0/config" should have numeric field "brightness_scale" = 254

  @id:MQTT-021
  Scenario: A lamp's config and its availability travel together
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
    And MQTT should have a retained "online" on "dali/ctl1/a0/vl/0/availability"

  @id:MQTT-022
  Scenario: The bulk announce pairs availability too
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/0/binding"
    Then the response status should be 200
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When I POST JSON {} to "/api/v1/settings/home-assistant/discovery-publish"
    Then the response status should be 202
    And the last operation eventually succeeds
    And MQTT should have a retained "online" on "dali/ctl1/a0/vl/0/availability"

  @id:MQTT-023
  Scenario: A second state commit does not rewrite the availability flag
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
    And MQTT should have a retained "online" on "dali/ctl1/a0/vl/0/availability"
    Given a successful target-state script for level 90 on short address 0
    When I PUT JSON {"power":"on","level":90} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And MQTT should have exactly 2 publishes on "dali/ctl1/a0/vl/0/state"
    And MQTT should have exactly 1 publish on "dali/ctl1/a0/vl/0/availability"

  @id:MQTT-025
  Scenario: A button press reaches Home Assistant as a named event
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When I PATCH JSON {"expose_input_devices":true} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    When a 24-bit input event frame 00 80 02 is observed on the bus
    Then the MQTT payload on "dali/ctl1/a0/in/0/0/state" should have string field "event_type" = "short_press"
    And the MQTT payload on "homeassistant/event/ctl1/a0_in0_0/config" should have string field "unique_id" = "ctl1_a0_in0_0"
