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
    And the JSON pointer "/now_ms" should be greater than 0

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

  @id:INP-086
  Scenario: A name over its 64 bytes is refused, not cut
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I PATCH JSON {"name":"ййййййййййййййййййййййййййййййййй"} to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

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

  @id:INP-085
  Scenario: Two writes to one instance are two operations, each with its own outcome
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock bus answers 24-bit query "01 00 83" with "00"
    And I PATCH JSON {"enabled":false} to "/api/v1/adapters/0/input-devices/0/instances/0" and the operation succeeds
    And I PATCH JSON {"enabled":false} to "/api/v1/adapters/0/input-devices/0/instances/0" and the operation succeeds
    And I send a GET request to "/api/v1/operations"
    Then the response status should be 200
    And the operations list should contain exactly 3 operations

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

  @id:INP-095
  Scenario: An event filter reads back every byte it set
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "0"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And the mock bus answers 24-bit query "01 FE 36" with "0F"
    And the mock bus answers 24-bit query "01 00 80" with "00"
    And the mock bus answers 24-bit query "01 00 90" with "0F"
    And the mock bus answers 24-bit query "01 00 91" with "80"
    And the mock bus answers 24-bit query "01 00 92" with "01"
    And I PATCH JSON {"event_filter":[15,128,1]} to "/api/v1/adapters/0/input-devices/0/instances/0" and the operation succeeds
    Then the mock transport 24-bit trace should be exactly "C1 32 01, C1 31 80, C1 30 0F, 01 FE 36, 01 00 68, 01 00 68, 01 00 80, 01 00 90, 01 00 91, 01 00 92"

  @id:INP-096
  Scenario: An event filter whose upper byte did not land is refused
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "0"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And the mock bus answers 24-bit query "01 FE 36" with "0F"
    And the mock bus answers 24-bit query "01 00 80" with "00"
    And the mock bus answers 24-bit query "01 00 90" with "0F"
    And the mock bus answers 24-bit query "01 00 91" with "7F"
    And I PATCH JSON {"event_filter":[15,128,1]} to "/api/v1/adapters/0/input-devices/0/instances/0"
    Then the response status should be 202
    And the last operation eventually fails
    And the operation error message should be "verify_failed"
    And the mock transport 24-bit trace should be exactly "C1 32 01, C1 31 80, C1 30 0F, 01 FE 36, 01 00 68, 01 00 68, 01 00 80, 01 00 90, 01 00 91"

  @id:INP-097
  Scenario: A push-button event filter reads back and keeps only the byte the instance has
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And the mock bus answers 24-bit query "01 FE 36" with "0F"
    And the mock bus answers 24-bit query "01 00 80" with "01"
    And the mock bus answers 24-bit query "01 00 90" with "0F"
    And I PATCH JSON {"event_filter":[15,128,1]} to "/api/v1/adapters/0/input-devices/0/instances/0" and the operation succeeds
    And I send a GET request to "/api/v1/adapters/0/input-devices/0"
    Then the response status should be 200
    And the JSON pointer "/instances/0/event_filter/value/0" should be 15
    And the JSON pointer "/instances/0/event_filter/value/1" should be 0
    And the JSON pointer "/instances/0/event_filter/value/2" should be 0
    And the mock transport 24-bit trace should be exactly "C1 32 01, C1 31 80, C1 30 0F, 01 FE 36, 01 00 68, 01 00 68, 01 00 80, 01 00 90"

  @id:INP-087
  Scenario Outline: An input-device route on an adapter that does not exist is 404
    When I send a <method> request to "<path>"
    Then the response status should be 404
    And the JSON error should be "not_found"
    And the mock transport should have sent no 24-bit frames

    Examples:
      | method | path                                         |
      | GET    | /api/v1/adapters/9/input-devices             |
      | POST   | /api/v1/adapters/9/input-devices/scan        |
      | POST   | /api/v1/adapters/9/input-devices/commission  |
      | GET    | /api/v1/adapters/9/input-devices/3           |
      | DELETE | /api/v1/adapters/9/input-devices/3           |

  @id:INP-088
  Scenario Outline: A short address outside 0..63 is not a resource id
    When I send a <method> request to "<path>"
    Then the response status should be 400
    And the JSON error should be "invalid_resource_id"

    Examples:
      | method | path                                           |
      | GET    | /api/v1/adapters/0/input-devices/64            |
      | DELETE | /api/v1/adapters/0/input-devices/64            |
      | POST   | /api/v1/adapters/0/input-devices/64/identify   |

  @id:INP-089
  Scenario Outline: A body field the route does not know is refused, not dropped
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1" and a corrected-map feedback answering capability 07 and colour capability 1F
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And I PATCH JSON <body> to "<path>"
    Then the response status should be 400
    And the JSON error should be "unknown_field"
    And the mock transport should have sent no 24-bit frames

    Examples:
      | path                                                 | body                                  |
      | /api/v1/adapters/0/input-devices/0                   | {"name":"panel","colour":"red"}       |
      | /api/v1/adapters/0/input-devices/0/instances/0       | {"event_scheme":2,"scheme":2}         |
      | /api/v1/adapters/0/input-devices/0/instances/0       | {"timers":{"t_long_ms":400}}          |
      | /api/v1/adapters/0/input-devices/0/instances/0/feedback | {"timing":3,"blink":true}          |

  @id:INP-090
  Scenario: Four instance groups are refused rather than cut to three
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And I PATCH JSON {"instance_groups":[1,2,3,4]} to "/api/v1/adapters/0/input-devices/0/instances/0"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And the mock transport should have sent no 24-bit frames

  @id:INP-091
  Scenario Outline: A commission body the route cannot honour is refused before the wire
    When I POST JSON <body> to "/api/v1/adapters/0/input-devices/commission"
    Then the response status should be <status>
    And the JSON error should be "<error>"
    And the mock transport should have sent no 24-bit frames

    Examples:
      | body                            | status | error         |
      | {"include_adressed":true}       | 400    | unknown_field |
      | {"include_addressed":"yes"}     | 422    | invalid_value |

  @id:INP-092
  Scenario Outline: A scan or identify body with a field is refused, the routes take none
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And I POST JSON {"force":true} to "<path>"
    Then the response status should be 400
    And the JSON error should be "unknown_field"
    And the mock transport should have sent no 24-bit frames

    Examples:
      | path                                          |
      | /api/v1/adapters/0/input-devices/scan         |
      | /api/v1/adapters/0/input-devices/0/identify   |

  @id:INP-093
  Scenario: Identify of an address nothing was scanned at is 404, whatever its body says
    When I POST JSON {"force":true} to "/api/v1/adapters/0/input-devices/5/identify"
    Then the response status should be 404
    And the JSON error should be "input_device_not_found"

  @id:INP-094
  Scenario Outline: A field the device reports but PATCH does not write is read-only, not unknown
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1" and a corrected-map feedback answering capability 07 and colour capability 1F
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    And I PATCH JSON <body> to "<path>"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"
    And the mock transport should have sent no 24-bit frames

    Examples:
      | path                                                    | body                          |
      | /api/v1/adapters/0/input-devices/0                      | {"present":false}             |
      | /api/v1/adapters/0/input-devices/0/instances/0          | {"instance_type":3}           |
      | /api/v1/adapters/0/input-devices/0/instances/0/feedback | {"probed":false}              |

  @id:INP-098
  Scenario Outline: Part 103 commissioning is refused while lamp commissioning runs on the adapter
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And the mock transport 24-bit trace is cleared
    Given the DALI transport blocks indefinitely
    When I POST JSON {"short_address":0} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 202
    And the last operation eventually runs
    When I POST JSON {} to "<path>"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON pointer "/message" should be "commissioning_active"
    And the operations list should contain exactly 3 operations
    When the DALI transport unblocks
    Then every operation eventually finishes
    And the mock transport should have sent no 24-bit frames

    Examples:
      | path                                        |
      | /api/v1/adapters/0/input-devices/commission |
      | /api/v1/adapters/0/input-devices/0/identify |

  @id:INP-099
  Scenario Outline: A second Part 103 commissioning on the adapter is refused while the first runs
    Given the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    Given the DALI transport blocks indefinitely
    When I POST JSON {} to "<running>"
    Then the response status should be 202
    And the last operation eventually becomes active
    When I POST JSON {} to "<refused>"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON pointer "/message" should be "commissioning_active"
    And the operations list should contain exactly 2 operations
    When the DALI transport unblocks
    Then every operation eventually finishes

    Examples:
      | running                                     | refused                                     |
      | /api/v1/adapters/0/input-devices/commission | /api/v1/adapters/0/input-devices/0/identify |
      | /api/v1/adapters/0/input-devices/0/identify | /api/v1/adapters/0/input-devices/commission |
