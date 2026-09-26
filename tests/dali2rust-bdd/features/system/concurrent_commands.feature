@stage-X1
Feature: Concurrent command handling
  As an API consumer
  I want to send multiple commands rapidly
  So that the system handles concurrent requests correctly

  @id:SYS-249
  Scenario: Two concurrent commands both receive correct responses
    Given the DALI transport responds with 0x42
    When I send two POST requests to "/api/v1/dali/command" concurrently
    Then both responses have status 200
    And both responses contain backward_frame 0x42

  @id:SYS-250
  Scenario: Rapid commands do not drop frames
    Given the DALI transport responds with 0x01
    When I send 5 DALI commands in sequence without delay
    Then all 5 responses have status 200
