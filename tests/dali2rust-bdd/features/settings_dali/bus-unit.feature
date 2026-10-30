@stage-R16
Feature: Settings — DALI: the controller as a Part 103 bus unit

  Another application controller on the segment may commission this one
  (IEC 62386-103 §9.2.3). What it changes on the wire becomes the
  controller's own setting, and the setting says the wire moved it.

  @id:SET-DALI-007
  Scenario: Another controller's SET SHORT ADDRESS becomes the controller's own address
    When a 24-bit frame C1 30 05 is observed on the bus
    And a 24-bit frame FF FE 14 is observed on the bus
    And a 24-bit frame FF FE 14 is observed on the bus
    Then the JSON pointer "/device_short_address" at "/api/v1/settings/dali" should eventually be "5"

  @id:SET-DALI-008
  Scenario: Another controller's DISABLE APPLICATION CONTROLLER makes this one passive
    When a 24-bit frame FF FE 17 is observed on the bus
    And a 24-bit frame FF FE 17 is observed on the bus
    Then the JSON pointer "/application_active" at "/api/v1/settings/dali" should eventually be "false"

  @id:SET-DALI-009
  Scenario: An operator's address replaces the one the wire set
    When a 24-bit frame C1 30 05 is observed on the bus
    And a 24-bit frame FF FE 14 is observed on the bus
    And a 24-bit frame FF FE 14 is observed on the bus
    Then the JSON pointer "/device_short_address" at "/api/v1/settings/dali" should eventually be "5"
    When I PATCH JSON {"device_short_address":9} to "/api/v1/settings/dali"
    Then the response status should be 200
    When a 24-bit frame C1 30 0B is observed on the bus
    And a 24-bit frame 13 FE 14 is observed on the bus
    And a 24-bit frame 13 FE 14 is observed on the bus
    Then the JSON pointer "/device_short_address" at "/api/v1/settings/dali" should eventually be "11"
