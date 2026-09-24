@stage-X1
Feature: DALI Level Command JSON API
  As a lighting controller
  I want to set luminaire level via a JSON endpoint
  So that I can control brightness via JSON without non-JSON bus wire on the internal path

  @id:DALI-020
  Scenario: Set level with short address
    Given a DALI mock transport with response 200
    When I send a JSON level command with wire_address 2 and level 254
    Then the response status should be 200
    And the JSON response success should be true

  @id:DALI-021
  Scenario: Set level with broadcast address
    Given a DALI mock transport with response 200
    When I send a JSON level command with wire_address 254 and level 128
    Then the response status should be 200
    And the JSON response success should be true

  @id:DALI-022
  Scenario: Set level with invalid JSON body
    Given a DALI mock transport with response 200
    When I POST invalid JSON "not-json" to "/api/v1/dali/level"
    Then the response status should be 400

  @id:DALI-023
  Scenario: DALI mock transport receives the forward frame for level command
    Given a DALI mock transport with no response
    When I send a JSON level command with wire_address 2 and level 100
    Then the response status should be 200
    And the DALI mock transport should have received 1 forward frame
