@stage-R1
Feature: The controller names its installation and its node

  @id:ADP-028
  Scenario: The controller summary names the installation by the Home Assistant controller id
    When I PATCH JSON {"controller_id":"ctl1"} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    When I send a GET request to "/api/v1/controller"
    Then the response status should be 200
    And the JSON field "controller_id" should be "ctl1"

  @id:ADP-029
  Scenario: A node names itself after its own hardware address
    Given the host stack runs on a network link with hardware address "02:00:00:a1:b2:c3", IPv4 address "10.0.0.5" and hostname "dali-a1b2c3"
    When I send a GET request to "/api/v1/controller"
    Then the response status should be 200
    And the JSON field "node_id" should be "dali-a1b2c3"
    And the JSON pointer "/network/hostname" should be "dali-a1b2c3"
    And the JSON pointer "/network/ip" should be "10.0.0.5"
    And the JSON pointer "/network/mac" should be "02:00:00:a1:b2:c3"

  @id:ADP-030
  Scenario: The installation takes the node's name until it is renamed, and the node keeps it
    Given the host stack runs on a network link with hardware address "02:00:00:a1:b2:c3", IPv4 address "10.0.0.5" and hostname "dali-a1b2c3"
    When I send a GET request to "/api/v1/controller"
    Then the JSON field "controller_id" should be "dali-a1b2c3"
    When I PATCH JSON {"controller_id":"ctl1"} to "/api/v1/settings/home-assistant"
    And I send a GET request to "/api/v1/controller"
    Then the JSON field "controller_id" should be "ctl1"
    And the JSON field "node_id" should be "dali-a1b2c3"
    And the JSON pointer "/network/hostname" should be "dali-a1b2c3"

  @id:ADP-031
  Scenario: A host stack without a network link has no node
    When I send a GET request to "/api/v1/controller"
    Then the response status should be 200
    And the JSON field "controller_id" should be "dali-controller"
    And the JSON pointer "/node_id" should be null
    And the JSON pointer "/network/hostname" should be null
    And the JSON pointer "/network/ip" should be null
    And the JSON pointer "/network/mac" should be null

  @id:ADP-032
  Scenario: The hostname is the one the interface carries, not one derived from the MAC
    Given the host stack runs on a network link with hardware address "02:00:00:a1:b2:c3", IPv4 address "10.0.0.5" and hostname "espressif"
    When I send a GET request to "/api/v1/controller"
    Then the response status should be 200
    And the JSON pointer "/network/hostname" should be "espressif"
    And the JSON field "node_id" should be "dali-a1b2c3"

  @id:ADP-033
  Scenario: A link without a lease has a node but no address
    Given the host stack runs on a network link with hardware address "02:00:00:a1:b2:c3", no IPv4 address and hostname "dali-a1b2c3"
    When I send a GET request to "/api/v1/controller"
    Then the response status should be 200
    And the JSON field "node_id" should be "dali-a1b2c3"
    And the JSON pointer "/network/ip" should be null
    And the JSON pointer "/network/mac" should be "02:00:00:a1:b2:c3"
