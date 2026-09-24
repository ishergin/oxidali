@stage-F2
Feature: Error handling
  As an API consumer
  I want proper error responses
  So that I can handle failures gracefully

  @id:SYS-008
  Scenario: Not found returns proper error
    When I send a GET request to "/api/v1/does-not-exist"
    Then the response status should be 404

  @id:SYS-009
  Scenario: Wrong method on health returns not found
    When I send a DELETE request to "/api/v1/health"
    Then the response status should be 404

  @id:SYS-050 @stage-X1
  Scenario: Health endpoint during command processing
    Given the DALI transport blocks indefinitely
    When I send a DALI command in background
    And I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the JSON HealthResponse status should be "ok"
    When the DALI transport unblocks
    And I drain the background request
