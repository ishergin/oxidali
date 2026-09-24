@stage-F3
Feature: Persistence slices host contract

  @id:PERS-002
  Scenario: Restart preserves adapter state when the same slice store is reused
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"name":"Round Trip","enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/adapters/0"
    Then the response status should be 200
    And the JSON field "name" should be "Round Trip"
    And the JSON boolean field "enabled" should be false

  @id:PERS-004
  Scenario: A schedule an operator entered survives a restart
    Given in-memory slice persistence is enabled for the host stack
    And HCL schedule "morning" exists
    When I restart the host stack
    And I send a GET request to "/api/v1/hcl-schedules/morning"
    Then the response status should be 200
    And the JSON field "algorithm" should be "stepped"
    And HCL schedule "morning" should have 1 target and 1 point

  @id:PERS-005
  Scenario: An identical attribute re-read writes nothing back to the slice store
    Given in-memory slice persistence is enabled for the host stack
    And a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I restart the host stack
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"name":"flush-fence-a"} to "/api/v1/adapters/0"
    Then the response status should be 200
    When I PATCH JSON {"name":"flush-fence-b"} to "/api/v1/adapters/0"
    Then the response status should be 200
    Then the diagnostics persistence flush total should settle at exactly 2
