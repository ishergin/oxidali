@stage-R6
Feature: Scene recall

  @id:SCN-080
  Scenario: Scene recall activates the scene only through the native broadcast recall
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    When I send a POST request to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 200
    And the JSON field "status" should be "confirmed"
    And the DALI mock transport should have sent only a broadcast go-to-scene 3 frame

  @id:SCN-081
  Scenario: Scene recall does not appear in operations
    Given a DALI mock transport with no response
    When I send a POST request to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 200
    And the response should carry a numeric correlation_id
    When I send a GET request to "/api/v1/operations"
    Then the operations list should be empty

  @id:SCN-082
  Scenario: Scene recall confirmation timeout returns 504
    Given a DALI mock transport with no response
    And the DALI transport blocks indefinitely
    When I send a POST request to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 504
    And the JSON error should be "confirmation_timeout"

  @id:SCN-086
  Scenario: Group-addressed recall sends exactly one group go-to-scene frame
    Given a DALI mock transport with no response
    When I POST JSON {"scope":"group","group_id":5} to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 200
    And the JSON field "status" should be "confirmed"
    And the DALI mock transport should have sent only a group 5 go-to-scene 3 frame

  @id:SCN-087
  Scenario: Group recall with an out-of-range group id is refused before the bus
    Given a DALI mock transport with no response
    When I POST JSON {"scope":"group","group_id":16} to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And no DALI frames should have been sent

  @id:SCN-088
  Scenario: Short-scoped recall is refused as an unsupported scope
    Given a DALI mock transport with no response
    When I POST JSON {"scope":"short"} to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And no DALI frames should have been sent

  @id:SCN-089
  Scenario: A recall body with a group id but no scope never falls back to broadcast
    Given a DALI mock transport with no response
    When I POST JSON {"group_id":7} to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And no DALI frames should have been sent

  @id:SCN-090
  Scenario: A recall body with a non-string scope never falls back to broadcast
    Given a DALI mock transport with no response
    When I POST JSON {"scope":5,"group_id":5} to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    And no DALI frames should have been sent

  @id:SCN-091
  Scenario: A recall body with an unknown key is refused, not silently accepted
    Given a DALI mock transport with no response
    When I POST JSON {"scope":"group","gruop_id":5} to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"
    And no DALI frames should have been sent
