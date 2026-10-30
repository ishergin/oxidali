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

  @id:SET-DALI-010
  Scenario: Another controller's quiescent mode holds back every forward frame until it stops
    When another controller sends START QUIESCENT MODE to every unit
    Then the JSON pointer "/quiescent" at "/api/v1/health" should eventually be "true"
    And the JSON pointer "/quiescent_remaining_ms" at "/api/v1/redundancy" should eventually be greater than 890000
    When I POST JSON {"wire_address":2,"level":100} to "/api/v1/dali/level"
    Then the response status should be 409
    And the JSON error should be "controller_quiescent"
    And the response header "Retry-After" should be "900"
    And the response header "X-Dali2rust-Role" should be "active"
    And the DALI mock transport should have received 0 forward frame
    When another controller sends STOP QUIESCENT MODE to every unit
    Then the JSON pointer "/quiescent" at "/api/v1/health" should eventually be "false"
    And the JSON pointer "/quiescent_remaining_ms" at "/api/v1/redundancy" should eventually be "null"
    When I POST JSON {"wire_address":2,"level":100} to "/api/v1/dali/level"
    Then the response status should be 200
    And the DALI mock transport should have received 1 forward frame

  @id:SET-DALI-011
  Scenario: A standby that hears no peer takes no bus while another controller is quiescent
    Given this controller has stood down
    When another controller sends START QUIESCENT MODE to every unit
    Then the JSON pointer "/quiescent" at "/api/v1/health" should eventually be "true"
    When I PATCH JSON {"enabled":true,"role":"standby","boot_listen_ms":0,"probe_interval_ms":250,"takeover_after_missed":1} to "/api/v1/settings/redundancy"
    Then the response status should be 200
    And the JSON pointer "/probes/published" at "/api/v1/redundancy" should eventually be greater than 2
    When I send a GET request to "/api/v1/redundancy"
    Then the JSON boolean field "active" should be false
    And the JSON pointer "/probes/unowned" should be 0
    And the DALI mock transport should have received 0 forward frame
    When another controller sends STOP QUIESCENT MODE to every unit
    Then the JSON pointer "/transitions/0/reason" at "/api/v1/redundancy" should eventually be "peer_silent"
    And the JSON pointer "/active" at "/api/v1/redundancy" should eventually be "true"
