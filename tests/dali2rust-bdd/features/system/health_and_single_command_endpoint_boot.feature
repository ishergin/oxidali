@stage-F3
Feature: Health and DALI command endpoints work in same server instance
  As a system operator
  I want both health and DALI command endpoints available after boot
  So that I can monitor and control the device simultaneously

  @id:SYS-010
  Scenario: Health endpoint responds before any DALI command
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the JSON HealthResponse status should be "ok"

  @id:SYS-011
  Scenario: DALI command works after health check
    Given a DALI mock transport with response 200
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true

  @id:SYS-012
  Scenario: Health endpoint still responds after DALI command
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the JSON HealthResponse status should be "ok"
