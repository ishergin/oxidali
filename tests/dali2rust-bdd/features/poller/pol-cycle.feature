@stage-I2
Feature: Poller cycle

  The poller is off by default, so every scenario turns it on through the
  production `PATCH /api/v1/settings/poller` surface and then waits on the
  `poller` block in `GET /api/v1/diagnostics` — counters, never wall clock.

  What a black-box client can see of a cycle is the counters, the operations
  list, and the runtime state a completed read leaves behind. Which attribute
  groups a read *carries* is a bus payload field no HTTP response exposes, so
  `POL-001`, `POL-003`, `POL-004`, `POL-005`, `POL-007` and `POL-009` are
  proved in `dali2rust-poller-runtime/tests/poller_cycle.rs` instead (same
  policy as the HCL scheduler's `SCN-064`).

  @id:POL-002
  Scenario: An unbound device is skipped when the setting asks for it
    Given adapter 0 has a discovered but unbound physical device 0
    When the poller is enabled with a 200 ms interval
    And the poller has run 2 more cycles
    Then the poller counter "targets_excluded" should have increased
    And the poller counter "reads_published" should be 0
    And no addressed DALI frames should have reached the bus

  @id:POL-006
  Scenario: Poller reads never create operation rows
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And the DALI mock answers every query with 128
    When the poller is enabled with a 200 ms interval
    And the poller has completed a read
    Then the operations list should contain no poller-created operation

  @id:POL-008
  Scenario: A poller read reaches the registry tagged as the poller's own
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And the DALI mock answers every query with 128
    When the poller is enabled with a 200 ms interval
    And the poller has completed a read
    Then virtual lamp 1 on adapter 0 should eventually report level 128 from value_source "poller"
