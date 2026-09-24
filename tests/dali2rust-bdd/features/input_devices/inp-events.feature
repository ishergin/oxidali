@stage-R16
Feature: IEC 62386-103 input events reach the bus

  @id:INP-001
  Scenario: A button press on the wire is decoded and counted as typed
    Given the DALI mock transport trace is cleared
    When a 24-bit input event frame 06 04 02 is observed on the bus
    Then diagnostics sniffer_translator input_events_typed should be at least 1
    And diagnostics sniffer_translator unknown_seen should be 0

  @id:INP-002
  Scenario: An event whose scheme carries no device identity is still published, and counted
    Given the DALI mock transport trace is cleared
    When a 24-bit input event frame 82 88 02 is observed on the bus
    Then diagnostics sniffer_translator input_events_ambiguous_scheme should be at least 1

  @id:INP-003
  Scenario: A power notification is counted separately from instance events
    Given the DALI mock transport trace is cleared
    When a 24-bit input event frame FE E0 45 is observed on the bus
    Then diagnostics sniffer_translator input_lifecycle should be at least 1
    And diagnostics sniffer_translator input_events_typed should be 0

  @id:INP-004
  Scenario: A 24-bit command is not an event and is not published as one
    Given the DALI mock transport trace is cleared
    When a 24-bit input event frame 07 FE 30 is observed on the bus
    Then diagnostics sniffer_translator unknown_seen should be at least 1
    And diagnostics sniffer_translator input_events_typed should be 0

  @id:INP-005
  Scenario: Decoding an input event publishes no DALI command
    Given the DALI mock transport trace is cleared
    When a 24-bit input event frame 06 04 02 is observed on the bus
    Then diagnostics sniffer_translator input_events_typed should be at least 1
    And the mock transport should have sent no frames

  @id:INP-006
  Scenario: A scheme-2 press is named in the input feed
    Given the DALI mock transport trace is cleared
    And the mock bus answers a control-device scan with a device at address 0 holding instance types "1"
    When input devices are scanned on adapter 0 and the scan succeeds
    And I open a WebSocket connection
    And the WebSocket client subscribes to "input"
    Then the WebSocket client should receive op "subscribed"
    When a 24-bit input event frame 00 80 02 is observed on the bus
    Then the WebSocket client should receive a "DaliInputEventObservedEvent" frame on channel "input"
    And the input event payload should name the event "short_press"
    And diagnostics sniffer_translator input_events_generic should be at least 1
    And diagnostics sniffer_translator input_events_typed should be 0
