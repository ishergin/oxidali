@stage-X1
Feature: Boot smoke test for health endpoint
  As a developer
  I want to verify basic boot state quickly
  So that regressions are caught early

  @id:SYS-001
  Scenario: Health endpoint reports ready after fresh boot
    Given a DALI mock transport with response 200
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the JSON HealthResponse status should be "ok"
    And the JSON HealthResponse version should not be empty
