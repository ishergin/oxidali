@stage-R2
Feature: Physical-device read surface is split into list, core, sections and banks

  @id:PD-200
  Scenario: The list carries a summary row, not the whole device
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices"
    Then the response status should be 200
    And the JSON pointer "/physical_devices/0/short_address" should be 0
    And the JSON pointer "/physical_devices/0/name" should be present
    And the JSON pointer "/physical_devices/0/device_type_effective" should be present
    And the JSON pointer "/physical_devices/0/color_mode_effective" should be present
    And the JSON pointer "/physical_devices/0/capabilities" should be present
    And the JSON pointer "/physical_devices/0/state" should be present
    And the JSON pointer "/physical_devices/0/attributes" should be absent
    And the JSON pointer "/physical_devices/0/memory_banks" should be absent
    And the JSON pointer "/now_ms" should be present
    And the JSON pointer "/physical_devices/0/now_ms" should be absent

  @id:PD-201
  Scenario: The detail carries the device core without sections or banks
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0"
    Then the response status should be 200
    And the JSON pointer "/short_address" should be 0
    And the JSON pointer "/device_type_effective" should be present
    And the JSON pointer "/capabilities" should be present
    And the JSON pointer "/state" should be present
    And the JSON pointer "/attributes" should be absent
    And the JSON pointer "/memory_banks" should be absent

  @id:PD-220
  Scenario: The attributes sub-resource answers every section by default
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0/attributes"
    Then the response status should be 200
    And the JSON pointer "/short_address" should be 0
    And the JSON pointer "/attributes/common_102" should be present
    And the JSON pointer "/attributes/memory_identity" should be present

  @id:PD-221
  Scenario: A sections filter emits only what was asked for
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0/attributes?sections=memory_identity"
    Then the response status should be 200
    And the JSON pointer "/attributes/memory_identity" should be present
    And the JSON pointer "/attributes/common_102" should be absent

  @id:PD-222
  Scenario: An unknown section name is refused rather than silently dropped
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0/attributes?sections=common_102,not_a_section"
    Then the response status should be 400
    And the JSON error should be "invalid_value"

  @id:PD-223
  Scenario: The attributes sub-resource 404s for a device that does not exist
    When I send a GET request to "/api/v1/adapters/0/physical-devices/9/attributes"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:PD-230
  Scenario: Bank coverage is its own resource
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/adapters/0/physical-devices/0/memory-banks"
    Then the response status should be 200
    And the JSON pointer "/short_address" should be 0
    And the JSON pointer "/memory_banks/0/bank" should be 0

  @id:PD-231
  Scenario: Bank coverage 404s for a device that does not exist
    When I send a GET request to "/api/v1/adapters/0/physical-devices/9/memory-banks"
    Then the response status should be 404
    And the JSON error should be "not_found"
