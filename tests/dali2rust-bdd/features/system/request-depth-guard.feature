@stage-X1
Feature: JSON request body depth guard
  As a controller facing hostile or broken clients
  I want deeply nested JSON bodies rejected before parsing
  So that request parsing cannot drive unbounded recursion on the httpd task

  @id:SYS-216
  Scenario: Deeply nested JSON body is rejected with invalid_json
    When I PATCH JSON {"name":[[[[[[[[[[1]]]]]]]]]]} to "/api/v1/adapters/0"
    Then the response status should be 400
    And the JSON error should be "invalid_json"

  @id:SYS-255
  Scenario: A body declared past the limit is refused before it is read, in the API's own shape
    When I POST a body declared as 70000 bytes but carrying 0 to "/api/v1/dali/level"
    Then the response status should be 413
    And the JSON error should be "payload_too_large"
    And the response header "X-Dali2rust-Role" should be "active"
