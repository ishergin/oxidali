@stage-I2
Feature: Power-cycle watch

  A gear that loses power comes back with its RAM state reset and its
  `powerCycleSeen` bit set (IEC 62386-102 §9.16.9). The bus-health probe ends
  with a broadcast QUERY POWER FAILURE, poller on or off; a YES behind an
  answered control sweeps the adapter's poll targets with runtime-status
  reads, and the registry forgets the RAM state of exactly the gear whose bit
  turned up set after a level command had cleared it.

  Each scenario restarts the controller with a short probe period and answers
  the probe's broadcast frames, and later each gear's status reads, from the
  mock's frame table, so background frames never consume a script. What the
  registry forgot is read back through the one fact a client can see: a lamp
  switched on without a level goes to the last active level the registry
  still holds.

  @id:PWR-001
  Scenario: The probe asks for power failures after its control, with the poller off
    Given the controller probes the bus every 300 ms
    When the controller has probed the bus 2 more times
    Then each probe should have broadcast the control, the lamp-failure query and the power-failure query in that order
    And no addressed DALI frames should have reached the bus
    And the poller counter "power_cycle_sweeps" should be 0

  @id:PWR-002
  Scenario: A YES sweeps the gear and forgets only the one that cycled
    Given the controller probes the bus every 300 ms
    And adapter 0 has discovered physical devices 0 and 1
    And virtual lamps 1 and 2 are bound to physical devices 0 and 1
    And virtual lamps 1 and 2 have been driven to level 100 and switched off
    When physical device 0 comes back from a power cycle
    And the controller has swept the bus for power cycles 1 time
    And virtual lamps 1 and 2 are switched on without a level
    Then virtual lamp 2 should eventually be on at level 100
    And virtual lamp 1 should eventually be on at no known level

  @id:PWR-003
  Scenario: A probe whose control goes unanswered is inconclusive and forgets nothing
    Given the controller probes the bus every 300 ms
    And adapter 0 has discovered physical devices 0 and 1
    And virtual lamps 1 and 2 are bound to physical devices 0 and 1
    And virtual lamps 1 and 2 have been driven to level 100 and switched off
    When physical device 0 comes back from a power cycle while the probe's control goes unanswered
    And the controller has probed the bus 2 more times
    And virtual lamps 1 and 2 are switched on without a level
    Then virtual lamp 1 should eventually be on at level 100
    And no gear should have been asked for its status
    And the poller counter "power_cycle_sweeps" should be 0

  @id:PWR-004
  Scenario: A standing YES sweeps once, not once per probe
    Given the controller probes the bus every 300 ms
    And adapter 0 has discovered physical devices 0 and 1
    And virtual lamps 1 and 2 are bound to physical devices 0 and 1
    When physical device 0 comes back from a power cycle
    And the controller has swept the bus for power cycles 1 time
    And the controller has probed the bus 3 more times
    Then the poller counter "power_cycle_sweeps" should be 1
    And the poller counter "power_cycle_sweep_reads" should be 2
    And every read the poller published should have been a power-cycle sweep read
    And each of physical devices 0 and 1 should have been asked for its status once

  @id:PWR-005
  Scenario: A gear driven after its power cycle is caught again when it cycles again
    Given the controller probes the bus every 300 ms
    And adapter 0 has discovered physical devices 0 and 1
    And virtual lamps 1 and 2 are bound to physical devices 0 and 1
    And virtual lamps 1 and 2 have been driven to level 100 and switched off
    When physical device 0 comes back from a power cycle
    And the controller has swept the bus for power cycles 1 time
    And virtual lamps 1 and 2 have been driven to level 100 and switched off
    And no gear reports a power cycle any more
    And the controller has probed the bus 2 more times
    And physical device 0 comes back from a power cycle
    And the controller has swept the bus for power cycles 2 times
    And virtual lamps 1 and 2 are switched on without a level
    Then virtual lamp 2 should eventually be on at level 100
    And virtual lamp 1 should eventually be on at no known level
