@stage-I2
Feature: Poller settings applied at runtime

  A settings change reaches the running poller as
  `PollerSettingsChangedEvent`; the worker swaps its local copy and keeps
  going. "No restart required" is observable here as the cycle counter never
  resetting across the change.

  @id:POL-030
  Scenario: A changed interval is picked up without a restart
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And the DALI mock answers every query with 128
    When the poller is enabled with a 200 ms interval
    And the poller has run 2 more cycles
    And I PATCH JSON {"interval_ms":3600000} to "/api/v1/settings/poller"
    Then the response status should be 200
    And the poller cycle counter should not have gone backwards
    And the poller should run no further cycle within its former interval

  @id:POL-031
  Scenario: Disabling the poller stops the reads without a restart
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And the DALI mock answers every query with 128
    When the poller is enabled with a 200 ms interval
    And the poller has completed a read
    And I PATCH JSON {"enabled":false} to "/api/v1/settings/poller"
    Then the response status should be 200
    And the poller should keep cycling without publishing further reads
