@stage-R16
Feature: The scan reads which variables of an instance are under local control (IEC 62386-333)

  A Part 333 device discards a configuration command for a variable whose manual
  configuration is enabled (§9.4.5.1). The scan asks each instance with
  QUERY MANUAL CONFIGURATION 103 and, where that answers 0, QUERY MANUAL CONFIGURATION 3xx
  (§11.12.5, §11.12.6), both with instance feature addressing. The state lands with the
  scan's feedback event, which also marks the instance's feedback probed.

  @id:INP-105
  Scenario: An instance with a Part 103 variable under local control reads active without a 3xx query
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    And the mock bus answers 24-bit query "01 20 0E" with "08"
    When input devices are scanned on adapter 0 and the scan succeeds
    Then the JSON pointer "/instances/0/manual_config_active" at "/api/v1/adapters/0/input-devices/0" should eventually be "true"
    And the mock transport 24-bit trace should not include "01 20 0F"

  @id:INP-106
  Scenario: An instance whose Part 103 and 3xx variables are all under bus control reads inactive
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    And the mock bus answers 24-bit query "01 20 0E" with "00"
    And the mock bus answers 24-bit query "01 20 0F" with "00"
    When input devices are scanned on adapter 0 and the scan succeeds
    Then the JSON pointer "/instances/0/manual_config_active" at "/api/v1/adapters/0/input-devices/0" should eventually be "false"

  @id:INP-107
  Scenario: A Part 3xx variable under local control alone makes the instance active
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    And the mock bus answers 24-bit query "01 20 0E" with "00"
    And the mock bus answers 24-bit query "01 20 0F" with "04"
    When input devices are scanned on adapter 0 and the scan succeeds
    Then the JSON pointer "/instances/0/manual_config_active" at "/api/v1/adapters/0/input-devices/0" should eventually be "true"

  @id:INP-108
  Scenario: A device silent to Part 333 leaves the state unknown and is not asked about 3xx
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    Then the JSON pointer "/instances/0/feedback/probed" at "/api/v1/adapters/0/input-devices/0" should eventually be "true"
    And the mock transport 24-bit trace should include "01 20 0E"
    And the mock transport 24-bit trace should not include "01 20 0F"
    When I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/0/manual_config_active" should be null

  @id:INP-109
  Scenario: Silence on the 3xx query after a Part 103 answer of 0 leaves the state unknown
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    And the mock bus answers 24-bit query "01 20 0E" with "00"
    When input devices are scanned on adapter 0 and the scan succeeds
    Then the JSON pointer "/instances/0/feedback/probed" at "/api/v1/adapters/0/input-devices/0" should eventually be "true"
    And the mock transport 24-bit trace should include "01 20 0F"
    When I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/0/manual_config_active" should be null
