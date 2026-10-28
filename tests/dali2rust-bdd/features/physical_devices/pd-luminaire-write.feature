@stage-R2
Feature: Physical-device luminaire data write

  @id:PD-291
  Scenario: A numeric field is written under an opened lock byte, the lock is put back and the bank is read back
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read script with DiiA Part 251 luminaire data for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the last operation eventually succeeds
    Given a luminaire-data write script for short address 0 storing year 24 that the gear keeps
    When I PATCH JSON {"year":24} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 202
    And the JSON field "type" should be "attribute_write"
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes luminaire year 24 with the rest of bank 1 intact
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-292
  Scenario: A text field is written null-terminated in field-aligned units
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read script with DiiA Part 251 luminaire data for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the last operation eventually succeeds
    Given a luminaire-data write script for short address 0 storing the colour "cool" that the gear keeps
    When I PATCH JSON {"luminaire_colour":"cool"} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 202
    And the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes the luminaire colour "cool"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-293
  Scenario: A read-back that disagrees fails the write and names the field
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read script with DiiA Part 251 luminaire data for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the last operation eventually succeeds
    Given a luminaire-data write script for short address 0 storing year 24 that the gear reads back unchanged
    When I PATCH JSON {"year":24} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "verify_failed"
    And the operation error message should be "year"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-294
  Scenario: A gear that does not confirm the write still gets its lock byte back
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read script with DiiA Part 251 luminaire data for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the last operation eventually succeeds
    Given a luminaire-data write script for short address 0 storing year 24 that the gear does not confirm
    When I PATCH JSON {"year":24} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "verify_unanswered"
    And the operation error message should be "year"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-295
  Scenario: A gear whose bank 1 was never read is refused before the wire
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    When I PATCH JSON {"year":24} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "bank1_not_part251"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-296
  Scenario: A field the gear's content format does not carry is refused before the wire
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read script with DiiA Part 251 luminaire data for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the last operation eventually succeeds
    When I PATCH JSON {"oem_name":"ACME"} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "bank1_field_not_in_format"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-297
  Scenario: The content format ID is read-only on this surface
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read script with DiiA Part 251 luminaire data for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the last operation eventually succeeds
    When I PATCH JSON {"content_format_id":4} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "bank1_field_read_only"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-298
  Scenario: A value the read side would not show back is refused
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read script with DiiA Part 251 luminaire data for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the last operation eventually succeeds
    When I PATCH JSON {"week":54} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    When I PATCH JSON {"luminaire_colour":"café"} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And all scripted DALI exchanges should be consumed without errors

  @id:PD-299
  Scenario: A gear whose bank 1 no longer declares Part 251 is refused on the wire, before any write
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read script with DiiA Part 251 luminaire data for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "profile"
    Then the last operation eventually succeeds
    Given a luminaire-data write script for short address 0 whose bank 1 header now declares a vendor format
    When I PATCH JSON {"year":24} to "/api/v1/adapters/0/physical-devices/0/luminaire-data"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error code should be "conflict"
    And the operation error message should be "bank1_not_part251"
    And all scripted DALI exchanges should be consumed without errors
