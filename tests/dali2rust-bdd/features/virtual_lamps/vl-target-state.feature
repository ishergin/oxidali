@stage-R4
Feature: Virtual lamp target state JSON validation

  @id:VL-050
  Scenario: PUT target-state rejects unknown field
    When I PUT JSON {"power":"on","extra":1} to "/api/v1/adapters/0/virtual-lamps/0/target-state"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:VL-051
  Scenario: PUT target-state rejects read-only status field
    When I PUT JSON {"status":{"raw":0}} to "/api/v1/adapters/0/virtual-lamps/0/target-state"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

  @id:VL-052
  Scenario: PUT target-state rejects unsupported color capability
    When I PUT JSON {"color_mode":"rgb","rgb":{"r":1,"g":2,"b":3}} to "/api/v1/adapters/0/virtual-lamps/0/target-state"
    Then the response status should be 422
    And the JSON error should be "unsupported_capability"

  @id:VL-053
  Scenario: PUT target-state for unbound lamp returns 200 without binding field
    When I PUT JSON {"power":"on","level":128} to "/api/v1/adapters/0/virtual-lamps/0/target-state"
    Then the response status should be 200
    And the JSON field "binding" should be absent

  @id:VL-054
  @stage-X1
  Scenario: Target-state sequence is retried once after contention retry exhaustion
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And a target-state script where the level command collides until a sequence retry succeeds
    When I PUT JSON {"power":"on","level":220} to "/api/v1/adapters/0/virtual-lamps/1/target-state"
    Then the response status should be 200
    And all scripted DALI exchanges should be consumed without errors
