@stage-R15
Feature: Commissioning address change

  @id:COMM-001
  Scenario: Address change accepts the request and creates a tracked operation
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an address-change script from short address 0 to 23
    When I POST JSON {"short_address":0,"new_short_address":23} to "/api/v1/adapters/0/commissioning/address-changes"
    Then the response status should be 202
    When I send a GET request to "/api/v1/operations"
    Then the response status should be 200
    And the operations list should contain a key starting with "comm-addr-"

  @id:COMM-004
  Scenario: Address change rejects identical source and target
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"short_address":0,"new_short_address":0} to "/api/v1/adapters/0/commissioning/address-changes"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-007
  Scenario: Address change rejects an unknown source physical device
    When I POST JSON {"short_address":9,"new_short_address":23} to "/api/v1/adapters/0/commissioning/address-changes"
    Then the response status should be 404
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-008
  Scenario: Address change rejects short addresses outside 0..63
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {"short_address":0,"new_short_address":64} to "/api/v1/adapters/0/commissioning/address-changes"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-010
  Scenario: Successful address change moves the record and reports both addresses
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an address-change script from short address 0 to 23
    When I POST JSON {"short_address":0,"new_short_address":23} to "/api/v1/adapters/0/commissioning/address-changes"
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors
    And the operation result old_short_address should be 0
    And the operation result new_short_address should be 23
    And physical device 23 should eventually exist on adapter 0
    And physical device 0 should eventually be absent on adapter 0
