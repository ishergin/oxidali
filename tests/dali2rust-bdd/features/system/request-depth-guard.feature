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
