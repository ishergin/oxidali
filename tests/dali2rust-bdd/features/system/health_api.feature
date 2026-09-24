@stage-F1
Feature: Health API
  As a monitoring system
  I want to check device health
  So that I know the device is operational

  @id:SYS-013
  Scenario: GET /api/v1/health returns ok
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the JSON HealthResponse status should be "ok"

  @id:SYS-014
  Scenario: POST /api/v1/health returns not found
    When I send a POST request to "/api/v1/health"
    Then the response status should be 404

  @id:SYS-015
  Scenario: Unknown route returns 404
    When I send a GET request to "/api/v1/nonexistent"
    Then the response status should be 404

  @id:SYS-016
  Scenario: Health response contains version
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the JSON HealthResponse version should not be empty
