@stage-R15
Feature: Commissioning replace device

  @id:COMM-055
  Scenario: Replacement rejects an unknown failed device
    When I POST JSON {"failed_short_address":9,"replacement_short_address":11} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 404
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-052
  Scenario: Replacement rejects an unknown replacement device
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"failed_short_address":0,"replacement_short_address":11} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 404
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-056
  Scenario: Replacement rejects identical failed and replacement addresses
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"failed_short_address":0,"replacement_short_address":0} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-057
  Scenario: Replacement refuses a request that would restore nothing
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"failed_short_address":0,"replacement_short_address":11,"restore":{"metadata_and_overrides":false,"attributes":false,"groups":false,"scenes":false}} to "/api/v1/adapters/0/commissioning/replacements"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frames
