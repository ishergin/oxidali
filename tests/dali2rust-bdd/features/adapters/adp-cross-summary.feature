@stage-R1
Feature: Controller summary reflects adapter composition

  @id:ADP-020
  Scenario: Controller summary reports adapter_count from composed host stack
    When I send a GET request to "/api/v1/controller"
    Then the response status should be 200
    And the JSON numeric field "adapter_count" should be 2
