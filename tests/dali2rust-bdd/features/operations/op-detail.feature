@stage-R7
Feature: Operation detail endpoint

  @id:OP-120
  Scenario: GET operation detail returns the current status for a discovery run
    When I POST JSON {"mode":"scan_known_short_addresses"} to "/api/v1/adapters/0/discovery-runs"
    Then the response status should be 202
    And the JSON field "operation_id" should be non-empty
    When I GET the operation from the last JSON response
    Then the response status should be 200
    And the JSON field "status" should be one of "accepted,running"

  @id:OP-121
  Scenario: GET operation detail returns 404 for an unknown id
    When I send a GET request to "/api/v1/operations/does-not-exist"
    Then the response status should be 404
    And the JSON error should be "not_found"
