@stage-R7
Feature: Operations list endpoint

  @id:OP-100
  Scenario: GET operations lists tracked operation keys and excludes request-scoped target-state correlations
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    Given the DALI mock transport trace is cleared
    When I send a GET request to "/api/v1/operations"
    Then the response status should be 200
    And the operations list should contain exactly 2 operations
    And the operations list should contain a key starting with "pd-disc-"
    And the operations list should contain a key starting with "pd-attr-"
    And the DALI mock transport should have received 0 forward frame

  @id:OP-101
  Scenario: GET operations returns an empty list when no tracked operations exist
    When I send a GET request to "/api/v1/operations"
    Then the response status should be 200
    And the operations list should be empty
