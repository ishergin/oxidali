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

  @id:SET-DALI-012
  Scenario: Another controller's SEND TESTFRAME puts the frame in DTR0–DTR2 on the wire at the priority it names
    When a 24-bit frame C1 30 07 is observed on the bus
    And a 24-bit frame C1 31 FE is observed on the bus
    And a 24-bit frame C1 32 10 is observed on the bus
    And a 24-bit frame C1 33 15 is observed on the bus
    And a 24-bit frame C1 32 11 is observed on the bus
    And a 24-bit frame C1 33 05 is observed on the bus
    Then the mock transport 24-bit trace should eventually be exactly "07 FE 10, 07 FE 10, 07 FE 10, 07 FE 11"
    And 24-bit forward frame 1 should be sent at priority 5
    And 24-bit forward frame 2 should be sent at priority 5
    And 24-bit forward frame 3 should be sent at priority 5

  @id:SET-DALI-013
  Scenario: A standby sends a test frame in a transaction, its repeat at priority 1
    Given this controller has stood down
    When a 24-bit frame C1 30 07 is observed on the bus
    And a 24-bit frame C1 31 FE is observed on the bus
    And a 24-bit frame C1 32 10 is observed on the bus
    And a 24-bit frame C1 33 4A is observed on the bus
    And a 24-bit frame C1 32 11 is observed on the bus
    And a 24-bit frame C1 33 05 is observed on the bus
    Then the mock transport 24-bit trace should eventually be exactly "07 FE 10, 07 FE 10, 07 FE 11"
    And 24-bit forward frame 1 should be sent at priority 2
    And 24-bit forward frame 2 should be sent at priority 1

  @id:SET-DALI-014
  Scenario: A 16-bit test frame carries DTR0 and DTR1 only
    When a 24-bit frame C1 30 A5 is observed on the bus
    And a 24-bit frame C1 31 5A is observed on the bus
    And a 24-bit frame C1 33 23 is observed on the bus
    And a 24-bit frame C1 32 11 is observed on the bus
    And a 24-bit frame C1 33 05 is observed on the bus
    Then the mock transport 24-bit trace should eventually be exactly "A5 5A 11"
    And the DALI mock transport should have received 1 forward frame
    And the DALI mock transport should have sent forward frame 0xA55A
    And forward frame 0xA55A should be sent at priority 3

  @id:SET-DALI-015
  Scenario: A controller another controller enabled for power cycle notification announces its next boot once at priority 2
    Given in-memory slice persistence is enabled for the host stack
    When a 24-bit frame FF FE 1F is observed on the bus
    And a 24-bit frame FF FE 1F is observed on the bus
    Then the JSON pointer "/power_cycle_notification" at "/api/v1/settings/dali" should eventually be "true"
    When I restart the host stack
    Then the mock transport 24-bit trace should be exactly "FE E0 00" within 6000 ms
    And 24-bit forward frame 1 should be sent at priority 2
    And the mock transport 24-bit trace should stay exactly "FE E0 00" for 5000 ms

  @id:SET-DALI-016
  Scenario: A controller at its factory power cycle notification setting announces no boot
    Given in-memory slice persistence is enabled for the host stack
    Then the JSON pointer "/power_cycle_notification" at "/api/v1/settings/dali" should eventually be "false"
    When I restart the host stack
    Then the mock transport 24-bit trace should stay exactly "" for 5500 ms

  @id:SET-DALI-017
  Scenario: An operator who enables power cycle notification makes the next boot announce the controller's address
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"device_short_address":5,"power_cycle_notification":true} to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON boolean field "power_cycle_notification" should be true
    When I restart the host stack
    Then the mock transport 24-bit trace should be exactly "FE E0 45" within 6000 ms
    And 24-bit forward frame 1 should be sent at priority 2
