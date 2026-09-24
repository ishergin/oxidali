@stage-F3
Feature: System workers run after boot
  As a system
  I want the HTTP and DALI path to start and coexist
  So that commands are processed after boot

  @id:SYS-002
  Scenario: Health endpoint responds after boot
    Given a DALI mock transport with response 200
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the JSON HealthResponse status should be "ok"

  @id:SYS-003
  Scenario: DALI worker processes command after boot
    Given a DALI mock transport with response 200
    When I send a JSON DALI command with address 1 and command 254
    Then the response status should be 200
    And the JSON DaliCommandResponse success should be true
    And the DALI mock transport should have received 1 forward frame

  @id:SYS-004
  Scenario: Health endpoint works while both workers run
    Given a DALI mock transport with response 200
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the JSON HealthResponse status should be "ok"
