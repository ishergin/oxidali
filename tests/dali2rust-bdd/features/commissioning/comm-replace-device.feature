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

  @id:COMM-057
  Scenario: Replacement refuses a request that would restore nothing
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"failed_short_address":0,"replacement_short_address":11,"restore":{"metadata_and_overrides":false,"attributes":false,"groups":false,"scenes":false}} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 422
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
    When I POST JSON {"failed_short_address":0,"replacement_short_address":1,"restore":{"metadata_and_overrides":true,"attributes":false,"groups":false,"scenes":false}} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the JSON pointer "/result/failed_short_address" should be 0
    And the JSON pointer "/result/replacement_short_address" should be 1
    And the JSON pointer "/result/restored/metadata_and_overrides" should be true
    And the JSON pointer "/result/restored/attributes" should be false
    And the JSON pointer "/result/restored/groups" should be false
    And the JSON pointer "/result/restored/scenes" should be false
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
    And no short address should have been programmed on the bus
    And physical device 1 should eventually exist on adapter 0
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON pointer "/random_address" should be 6036930
    When I send a GET request to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 200
    And the JSON pointer "/binding/physical_short_address" should be 0
