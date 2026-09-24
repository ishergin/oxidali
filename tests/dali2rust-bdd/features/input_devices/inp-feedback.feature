@stage-R16
Feature: Part 332 feedback: configuration over REST, probed dialect, timers

  The panel's indicator is a feature of its own instance (IEC 62386-332), not a
  control-gear channel. REST exposes only the NVM configuration; driving the
  LEDs (ACTIVATE/STOP/SELECT) belongs to the rules engine and is proved at the
  crate layer. The query opcode map moved between editions (DiiA(SW)098bp
  §11.6.1): the corrected map answers 0x47..0x4F, the 2017 one 0x27..0x2F, and
  which dialect a panel speaks is probed at scan, recorded, and then used.

  @id:INP-070
  Scenario: A feedback write is DTR0-proved, sent twice and read back
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1" and a corrected-map feedback answering capability 07 and colour capability 1F
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And the mock bus answers 24-bit query "01 FE 36" with "C8"
    And the mock bus answers 24-bit query "01 20 4C" with "C8"
    And I PATCH JSON {"active_brightness":200} to "/api/v1/adapters/0/input-devices/0/instances/0/feedback" and the operation succeeds
    Then the mock transport 24-bit trace should be exactly "C1 30 C8, 01 FE 36, 01 20 13, 01 20 13, 01 20 4C"

  @id:INP-071
  Scenario: A confirmed feedback write lands in the registry with provenance
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1" and a corrected-map feedback answering capability 07 and colour capability 1F
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock bus answers 24-bit query "01 FE 36" with "C8"
    And the mock bus answers 24-bit query "01 20 4C" with "C8"
    And I PATCH JSON {"active_brightness":200} to "/api/v1/adapters/0/input-devices/0/instances/0/feedback" and the operation succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 200
    And the JSON pointer "/instances/0/feedback/present" should be true
    And the JSON pointer "/instances/0/feedback/opcode_map" should be "diia_corrected"
    And the JSON pointer "/instances/0/feedback/active_brightness" should be 200

  @id:INP-072
  Scenario: A colour outside 1..63 is refused before the wire
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1" and a corrected-map feedback answering capability 07 and colour capability 1F
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And I PATCH JSON {"active_colour":64} to "/api/v1/adapters/0/input-devices/0/instances/0/feedback"
    Then the response status should be 422
    And the mock transport should have sent no 24-bit frames

  @id:INP-073
  Scenario: The 2017 opcode dialect is probed, recorded and then used for readbacks
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1" and an ed1-map feedback answering capability 03
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/0/feedback/opcode_map" should be "ed1"
    When the mock transport 24-bit trace is cleared
    And the mock bus answers 24-bit query "01 FE 36" with "28"
    And the mock bus answers 24-bit query "01 20 2D" with "28"
    And I PATCH JSON {"timing":40} to "/api/v1/adapters/0/input-devices/0/instances/0/feedback" and the operation succeeds
    Then the mock transport 24-bit trace should be exactly "C1 30 28, 01 FE 36, 01 20 12, 01 20 12, 01 20 2D"

  @id:INP-074
  Scenario: Scan records feedback absence honestly
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 200
    And the JSON pointer "/instances/0/feedback/present" should be false

  @id:INP-075
  Scenario: A feedback write to an instance without the feature fails with a named cause
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I PATCH JSON {"active_brightness":10} to "/api/v1/adapters/0/input-devices/0/instances/0/feedback"
    Then the response status should be 422
    And the response body should contain "feedback_not_supported"

  @id:INP-076
  Scenario: A timer write reads the ROM minimum first and proves the value
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And the mock bus answers 24-bit query "01 00 0B" with "05"
    And the mock bus answers 24-bit query "01 FE 36" with "19"
    And the mock bus answers 24-bit query "01 00 0A" with "19"
    And I PATCH JSON {"timers":{"t_short_ms":500}} to "/api/v1/adapters/0/input-devices/0/instances/0" and the operation succeeds
    Then the mock transport 24-bit trace should be exactly "01 00 0B, C1 30 19, 01 FE 36, 01 00 00, 01 00 00, 01 00 0A"
    When I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/0/timers/0/value" should be 25

  @id:INP-077
  Scenario: A timer below the device's ROM minimum fails the operation, not silently
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock bus answers 24-bit query "01 00 0B" with "1E"
    And I PATCH JSON {"timers":{"t_short_ms":100}} to "/api/v1/adapters/0/input-devices/0/instances/0"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error message should be "below_device_minimum"

  @id:INP-078
  Scenario: A timer outside the static range is refused before the wire
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And I PATCH JSON {"timers":{"t_short_ms":6000}} to "/api/v1/adapters/0/input-devices/0/instances/0"
    Then the response status should be 422
    And the mock transport should have sent no 24-bit frames

  @id:INP-079
  Scenario: A common-brightness device invalidates the sibling's feedback cache on write
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1,1" and a corrected-map feedback answering capability 47 and colour capability 1F
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock bus answers 24-bit query "01 FE 36" with "64"
    And the mock bus answers 24-bit query "01 21 4C" with "64"
    And I PATCH JSON {"active_brightness":100} to "/api/v1/adapters/0/input-devices/0/instances/1/feedback" and the operation succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/1/feedback/active_brightness" should be 100
    When the mock bus answers 24-bit query "01 FE 36" with "C8"
    And the mock bus answers 24-bit query "01 20 4C" with "C8"
    And I PATCH JSON {"active_brightness":200} to "/api/v1/adapters/0/input-devices/0/instances/0/feedback" and the operation succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/0/feedback/active_brightness" should be 200
    And the JSON pointer "/instances/1/feedback/active_brightness" should be null

  @id:INP-080
  Scenario: A readback nobody answers is reported as unanswered, not as a refused write
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1" and a corrected-map feedback answering capability 07 and colour capability 1F
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock bus answers 24-bit query "01 FE 36" with "C8"
    And I PATCH JSON {"active_brightness":200} to "/api/v1/adapters/0/input-devices/0/instances/0/feedback"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error message should be "verify_unanswered"

  @id:INP-081
  Scenario: A readback that answers a different value is the write being refused
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1" and a corrected-map feedback answering capability 07 and colour capability 1F
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock bus answers 24-bit query "01 FE 36" with "C8"
    And the mock bus answers 24-bit query "01 20 4C" with "05"
    And I PATCH JSON {"active_brightness":200} to "/api/v1/adapters/0/input-devices/0/instances/0/feedback"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error message should be "verify_failed"
