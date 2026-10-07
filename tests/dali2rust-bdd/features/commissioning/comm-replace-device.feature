@stage-R15
Feature: Commissioning replace device

  @id:COMM-055
  Scenario: Replacement rejects an unknown failed device
    When I POST JSON {"failed_short_address":9,"replacement_short_address":11} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 404
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-052
  Scenario: Replacement rejects an unknown replacement device
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"failed_short_address":0,"replacement_short_address":11} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 404
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-056
  Scenario: Replacement rejects identical failed and replacement addresses
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"failed_short_address":0,"replacement_short_address":0} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-114
  Scenario: Replacement refuses restore keys for slices the apply programs
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"failed_short_address":0,"replacement_short_address":11,"restore":{"metadata_and_overrides":true,"groups":true}} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 400
    And the JSON error should be "unknown_field"
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-100
  Scenario: A replacement takes over the failed device's address and its lamp keeps its Home Assistant identity
    Given adapter 0 has discovered a colour-temperature device 0 and an RGB device 1
    When I PATCH JSON {"name":"Hall"} to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    Then MQTT should have exactly 1 publish on "homeassistant/light/ctl1/a0_vl_1/config"
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_1/config" should have string field "unique_id" = "ctl1_a0_vl_1"
    Given a replacement script in which short address 0 stays silent and short address 1 takes its address
    When I POST JSON {"failed_short_address":0,"replacement_short_address":1,"restore":{"metadata_and_overrides":true}} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the JSON pointer "/result/failed_short_address" should be 0
    And the JSON pointer "/result/replacement_short_address" should be 1
    And the JSON pointer "/result/restored/metadata_and_overrides" should be true
    And the JSON pointer "/result/restored/attributes" should be absent
    And the JSON pointer "/result/restored/groups" should be absent
    And the JSON pointer "/result/restored/scenes" should be absent
    And all scripted DALI exchanges should be consumed without errors
    And the transport should have carried exactly the replacement of short address 0 by short address 1
    And physical device 1 should eventually be absent on adapter 0
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON pointer "/random_address" should be 2756371
    And the JSON pointer "/name" should be "Hall"
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 200
    And the JSON pointer "/binding/physical_short_address" should be 0
    And MQTT should have exactly 2 publishes on "homeassistant/light/ctl1/a0_vl_1/config"
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_1/config" should have string field "unique_id" = "ctl1_a0_vl_1"
    And the MQTT payload on "homeassistant/light/ctl1/a0_vl_1/config" array field "supported_color_modes" should contain "rgb"

  @id:COMM-101
  Scenario: A replacement whose failed device still answers fails verify_failed and moves nothing
    Given adapter 0 has discovered a colour-temperature device 0 and an RGB device 1
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    Given a replacement script in which short address 0 still answers
    When I POST JSON {"failed_short_address":0,"replacement_short_address":1} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "verify_failed"
    And all scripted DALI exchanges should be consumed without errors
    And the transport should have carried exactly the identity probe of short address 0
    And physical device 1 should eventually exist on adapter 0
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON pointer "/random_address" should be 6036930
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 200
    And the JSON pointer "/binding/physical_short_address" should be 0

  @id:COMM-110
  Scenario: The record keeps the replacement's own groups and scenes, so the next applies program it
    Given adapter 0 has discovered a colour-temperature device 0 and an RGB device 1
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    Given adapter 0 desired membership includes virtual lamp 1 in group 7
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors
    Given a replacement script in which short address 0 stays silent and short address 1 takes its address
    When I POST JSON {"failed_short_address":0,"replacement_short_address":1} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the transport should have carried exactly the replacement of short address 0 by short address 1
    And physical device 1 should eventually be absent on adapter 0
    Given a DALI mock transport with no response
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors

  @id:COMM-111
  Scenario: A replacement is refused before the wire while both addresses carry a lamp
    Given adapter 0 has discovered a colour-temperature device 0 and an RGB device 1
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I PUT JSON {"physical_short_address":1} to "/api/v1/adapters/0/virtual-lamps/2/binding"
    Then the response status should be 200
    Given the DALI mock transport trace is cleared
    When I POST JSON {"failed_short_address":0,"replacement_short_address":1} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON pointer "/message" should be "replacement_bound_to_another_lamp"
    And the DALI mock transport should have received 0 forward frames
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/2"
    Then the response status should be 200
    And the JSON pointer "/binding/physical_short_address" should be 1

  @id:COMM-113
  Scenario: A WebSocket client hears both addresses and the lamp of a replacement
    Given adapter 0 has discovered a colour-temperature device 0 and an RGB device 1
    When I PUT JSON {"physical_short_address":1} to "/api/v1/adapters/0/virtual-lamps/2/binding"
    Then the response status should be 200
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "physical_devices,virtual_lamps"
    And a replacement script in which short address 0 stays silent and short address 1 takes its address
    When I POST JSON {"failed_short_address":0,"replacement_short_address":1} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the WebSocket client should receive a "PhysicalDeviceChangedEvent" frame on channel "physical_devices" whose payload "short_address" is 0
    And the WebSocket client should receive a "PhysicalDeviceChangedEvent" frame on channel "physical_devices" whose payload "short_address" is 1
    And the WebSocket client should receive a "VirtualLampChangedEvent" frame on channel "virtual_lamps" whose payload "virtual_lamp_id" is 2
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/2"
    Then the response status should be 200
    And the JSON pointer "/binding/physical_short_address" should be 0
