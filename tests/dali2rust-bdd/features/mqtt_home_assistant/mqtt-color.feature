@stage-I4
Feature: MQTT/HA — colour speaks Home Assistant's own dialect
  As a Home Assistant user dragging a colour-temperature slider
  I want the bridge to read and write the JSON-schema `color_temp` key in kelvin
  So that colour works in both directions instead of being silently ignored

  @id:MQTT-012
  Scenario: A colour-temperature command on a lamp topic stages DT8 colour
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/0/binding"
    Then the response status should be 200
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a cct 3000K target-state script for short address 0
    When Home Assistant publishes {"color_temp":3000} on "dali/ctl1/a0/vl/0/set"
    Then adapter 0 physical device 0 eventually exposes runtime colour temperature 3000 K
    And the scripted DALI exchanges should eventually be consumed

  @id:MQTT-013
  Scenario: Lamp state publishes `color_temp` in kelvin and never the legacy key
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
    Given a cct 3000K target-state script for short address 0
    When I PUT JSON {"color_mode":"cct","color_temperature_kelvin":3000} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    And MQTT should have exactly 2 publishes on "dali/ctl1/a0/vl/0/state"
    And the MQTT payload on "dali/ctl1/a0/vl/0/state" should have numeric field "color_temp" = 3000
    And the MQTT payload on "dali/ctl1/a0/vl/0/state" should have string field "color_mode" = "color_temp"
    And the MQTT payload on "dali/ctl1/a0/vl/0/state" should not have field "color_temp_kelvin"
    And the MQTT payload on "dali/ctl1/a0/vl/0/state" should have string field "state" = "ON"

  @id:MQTT-015
  Scenario: Discovery declares kelvin colour temperature and the lamp's colour modes
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
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_0/config" should have boolean field "color_temp_kelvin" = true
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_0/config" array field "supported_color_modes" should contain "color_temp"

  @id:MQTT-016
  Scenario: A colour-temperature command on a group topic stages and activates group-addressed DT8 colour
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a group cct 3000K target-state script for group 7
    When Home Assistant publishes {"color_temp":3000} on "dali/ctl1/a0/group/7/set"
    Then the scripted DALI exchanges should eventually be consumed

  @id:MQTT-020
  Scenario: A group colour command carrying state ON also switches the group on
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    And a group cct 3000K power-on target-state script for group 7
    When Home Assistant publishes {"state":"ON","color_temp":3000} on "dali/ctl1/a0/group/7/set"
    Then the scripted DALI exchanges should eventually be consumed
