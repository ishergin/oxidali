@stage-R16
Feature: The scan reads which variables of an instance are under local control (IEC 62386-333)

  A Part 333 device discards a configuration command for a variable whose manual
  configuration is enabled (§9.4.5.1). The scan asks each instance with
  QUERY MANUAL CONFIGURATION 103 and, where that answers, QUERY MANUAL CONFIGURATION 3xx
  (§11.12.5, §11.12.6), both with instance feature addressing.

  @id:INP-105
  Scenario: An instance with a Part 103 variable under local control reads active
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    And the mock bus answers 24-bit query "01 20 0E" with "08"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/0/manual_config_active" should be true

  @id:INP-106
  Scenario: An instance whose variables are all under bus control reads inactive
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    And the mock bus answers 24-bit query "01 20 0E" with "00"
    And the mock bus answers 24-bit query "01 20 0F" with "00"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/0/manual_config_active" should be false

  @id:INP-107
  Scenario: A Part 3xx variable under local control alone makes the instance active
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    And the mock bus answers 24-bit query "01 20 0E" with "00"
    And the mock bus answers 24-bit query "01 20 0F" with "04"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/0/manual_config_active" should be true

  @id:INP-108
  Scenario: A device silent to Part 333 leaves the state unknown and is not asked about 3xx
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/instances/0/manual_config_active" should be null
    And the mock transport 24-bit trace should include "01 20 0E"
    And the mock transport 24-bit trace should not include "01 20 0F"
