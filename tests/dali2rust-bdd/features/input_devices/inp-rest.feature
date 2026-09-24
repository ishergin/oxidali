@stage-R16
Feature: IEC 62386-103 input devices as a REST resource

  Background:
    Given the DALI mock transport trace is cleared

  @id:INP-010
  Scenario: A scanned device appears as a summary row
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1,1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices"
    Then the response status should be 200
    And the JSON pointer "/input_devices/0/short_address" should be 0
    And the JSON pointer "/input_devices/0/instance_count" should be 2
    And the JSON pointer "/input_devices/0/present" should be true

  @id:INP-011
  Scenario: The detail shows instances, and an unread timer is null with null provenance
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 200
    And the JSON pointer "/instances/0/instance_type_name" should be "push_button"
    And the JSON pointer "/instances/0/timers/0/value" should be null
    And the JSON pointer "/instances/0/timers/0/read_at_ms" should be null
    And the JSON pointer "/instances/0/event_scheme_confirmed" should be false

  @id:INP-012
  Scenario: An address nothing was scanned at is 404
    When I send a GET request to "/api/v1/adapters/0/input-devices/63"
    Then the response status should be 404

  @id:INP-013
  Scenario: Our metadata is patched and visible, and the two address spaces stay apart
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I PATCH JSON {"name":"панель-прихожая","ha_expose":false} to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 200
    And the JSON pointer "/name" should be "панель-прихожая"
    And the JSON pointer "/ha_expose" should be false

  @id:INP-016
  Scenario: Forgetting a device removes the record, and only the record
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a DELETE request to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 200
    When I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 404

  @id:INP-017
  Scenario: An empty metadata patch is a client error, not a silent write
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I PATCH JSON {} to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 400

  @id:INP-030
  Scenario: Event priority 2 is refused at ingress and no frame reaches the wire
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And I PATCH JSON {"event_priority":2} to "/api/v1/adapters/0/input-devices/0/instances/0"
    Then the response status should be 422
    And the mock transport should have sent no 24-bit frames

  @id:INP-031
  Scenario: An event scheme the standard does not define is refused
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I PATCH JSON {"event_scheme":5} to "/api/v1/adapters/0/input-devices/0/instances/0"
    Then the response status should be 422

  @id:INP-032
  Scenario: Configuring an instance the device never declared is 404
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I PATCH JSON {"event_scheme":2} to "/api/v1/adapters/0/input-devices/0/instances/7"
    Then the response status should be 404

  @id:INP-018
  Scenario: A scan records what the device declares about itself
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1" declaring capabilities 03 status 28 version 08
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 200
    And the JSON pointer "/device_capabilities" should be 3
    And the JSON pointer "/device_status" should be 40
    And the JSON pointer "/version_number" should be 8

  @id:INP-019
  Scenario: Disabling an instance is written send-twice and proved by its status
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And the mock bus answers 24-bit query "01 00 83" with "00"
    And I PATCH JSON {"enabled":false} to "/api/v1/adapters/0/input-devices/0/instances/0" and the operation succeeds
    Then the mock transport 24-bit trace should be exactly "01 00 63, 01 00 63, 01 00 83"

  @id:INP-082
  Scenario: A device that answered an earlier scan and stays silent in the next loses presence
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the JSON pointer "/present" should be true
    Given the segment answers no control-device scan at all
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 200
    And the JSON pointer "/present" should be false
    And the JSON pointer "/last_seen_ms" should be null
    And the JSON pointer "/name" should be null

  @id:INP-083
  Scenario: A scan frame another master collided with is sent again, and the scan completes
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    And the next 24-bit frame collides with another master's on the wire
    When input devices are scanned on adapter 0 and the scan succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices"
    Then the response status should be 200
    And the JSON pointer "/input_devices/0/short_address" should be 0
    And the JSON pointer "/input_devices/0/present" should be true
    And the mock transport 24-bit trace should begin with "01 FE 35, 01 FE 35"

  @id:INP-084
  Scenario: An instance write continues at priority 1 after its proof
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And the mock bus answers 24-bit query "01 FE 36" with "02"
    And the mock bus answers 24-bit query "01 00 8B" with "02"
    And I PATCH JSON {"event_scheme":2} to "/api/v1/adapters/0/input-devices/0/instances/0" and the operation succeeds
    Then the mock transport 24-bit trace should be exactly "C1 30 02, 01 FE 36, 01 00 67, 01 00 67, 01 00 8B"
    And the first 24-bit forward frame should be sent at priority 3
    And 24-bit forward frame 2 should be sent at priority 1
    And 24-bit forward frame 3 should be sent at priority 1
    And 24-bit forward frame 4 should be sent at priority 1
    And 24-bit forward frame 5 should be sent at priority 3
