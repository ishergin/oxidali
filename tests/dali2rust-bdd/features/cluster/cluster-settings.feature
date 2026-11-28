@stage-I6
Feature: Cluster — node settings

  What one controller stores about the cluster it belongs to: whether it takes
  part, the cluster's name, its node number, the port, the DALI wire it shares
  and what it forwards and accepts. The cluster is off until it is switched on
  and needs a name before it is. Rules that join two fields are checked against
  the stored settings and the patch together, never against the patch alone.

  @id:CLU-001
  Scenario: GET returns the documented defaults before any PATCH
    When I send a GET request to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be false
    And the JSON field "cluster_name" should be ""
    And the JSON numeric field "node_number" should be 0
    And the JSON numeric field "port" should be 47810
    And the JSON pointer "/wire_same_as" should be null
    And the JSON boolean field "forward_operator" should be true
    And the JSON boolean field "forward_home_assistant" should be true
    And the JSON boolean field "forward_hcl" should be false
    And the JSON boolean field "forward_foreign_master" should be false
    And the JSON boolean field "accept_all_lamps" should be false
    And the JSON boolean field "send_all_lamps" should be false
    And the JSON boolean field "accept_single_lamps" should be false
    And the JSON boolean field "wire_protection" should be true
    And the JSON boolean field "share_input_events" should be true
    And the JSON boolean field "read_only" should be false
    And the JSON pointer "/block/first" should be 0
    And the JSON pointer "/block/last" should be 15

  @id:CLU-002
  Scenario: PATCH round-trips every field through a subsequent GET
    When I PATCH JSON {"cluster_name":"dali-cluster","node_number":3,"port":47900,"wire_same_as":1,"forward_operator":false,"forward_home_assistant":false,"forward_hcl":true,"forward_foreign_master":true,"accept_all_lamps":true,"send_all_lamps":true,"accept_single_lamps":true,"share_input_events":false} to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON numeric field "node_number" should be 3
    When I send a GET request to "/api/v1/cluster"
    Then the JSON field "cluster_name" should be "dali-cluster"
    And the JSON numeric field "node_number" should be 3
    And the JSON numeric field "port" should be 47900
    And the JSON pointer "/wire_same_as" should be 1
    And the JSON boolean field "forward_operator" should be false
    And the JSON boolean field "forward_home_assistant" should be false
    And the JSON boolean field "forward_hcl" should be true
    And the JSON boolean field "forward_foreign_master" should be true
    And the JSON boolean field "accept_all_lamps" should be true
    And the JSON boolean field "send_all_lamps" should be true
    And the JSON boolean field "accept_single_lamps" should be true
    And the JSON boolean field "wire_protection" should be true
    And the JSON boolean field "share_input_events" should be false
    And the JSON boolean field "enabled" should be false
    And the JSON pointer "/block/first" should be 48
    And the JSON pointer "/block/last" should be 63

  @id:CLU-003
  Scenario: The cluster cannot be switched on without a name
    When I PATCH JSON {"enabled":true} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "cluster_name_required"
    When I send a GET request to "/api/v1/cluster"
    Then the JSON boolean field "enabled" should be false

  @id:CLU-004
  Scenario: A name and the switch in one PATCH turn the cluster on, and the controller says so
    When I send a GET request to "/api/v1/controller"
    Then the JSON pointer "/cluster/enabled" should be false
    When I PATCH JSON {"enabled":true,"cluster_name":"dali-cluster"} to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be true
    When I send a GET request to "/api/v1/controller"
    Then the JSON pointer "/cluster/enabled" should be true

  @id:CLU-005
  Scenario: An enabled cluster keeps its name
    When I PATCH JSON {"enabled":true,"cluster_name":"dali-cluster"} to "/api/v1/cluster"
    Then the response status should be 200
    When I PATCH JSON {"cluster_name":""} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_name_required"
    When I PATCH JSON {"enabled":false,"cluster_name":""} to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON field "cluster_name" should be ""

  @id:CLU-006
  Scenario Outline: PATCH refuses a value outside its domain
    When I PATCH JSON <body> to "/api/v1/cluster"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

    Examples:
      | body                                                                                    |
      | {"node_number":8}                                                                       |
      | {"node_number":-1}                                                                      |
      | {"port":0}                                                                              |
      | {"port":65536}                                                                          |
      | {"wire_same_as":8}                                                                      |
      | {"wire_same_as":"own"}                                                                  |
      | {"enabled":"yes"}                                                                       |
      | {"share_input_events":1}                                                                |
      | {"cluster_name":7}                                                                      |
      | {"cluster_name":"0123456789012345678901234567890123456789012345678901234567890123456"} |

  @id:CLU-007
  Scenario Outline: PATCH refuses a field it does not take
    When I PATCH JSON <body> to "/api/v1/cluster"
    Then the response status should be <status>
    And the JSON error should be "<error>"

    Examples:
      | body                      | status | error             |
      | {"multicast":true}        | 400    | unknown_field     |
      | {"read_only":true}        | 422    | unsupported_field |
      | {"block":{"first":16}}    | 422    | unsupported_field |

  @id:CLU-008
  Scenario: A shared wire names another node, never this one
    When I PATCH JSON {"wire_same_as":0} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON field "message" should be "wire_names_this_node"
    When I PATCH JSON {"node_number":1,"wire_same_as":2} to "/api/v1/cluster"
    Then the response status should be 200
    When I PATCH JSON {"node_number":2} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON field "message" should be "wire_names_this_node"
    When I PATCH JSON {"wire_same_as":null} to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON pointer "/wire_same_as" should be null

  @id:CLU-009
  Scenario: Loop protection cannot be off while the foreign master is forwarded on a shared wire
    When I PATCH JSON {"wire_same_as":1,"forward_foreign_master":true,"wire_protection":false} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON field "message" should be "wire_protection_required"
    When I PATCH JSON {"wire_same_as":1,"forward_foreign_master":true} to "/api/v1/cluster"
    Then the response status should be 200
    When I PATCH JSON {"wire_protection":false} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON field "message" should be "wire_protection_required"
    When I send a GET request to "/api/v1/cluster"
    Then the JSON boolean field "wire_protection" should be true

  @id:CLU-010
  Scenario: Loop protection may be off on a wire this node keeps to itself, or when the foreign master stays home
    When I PATCH JSON {"forward_foreign_master":true,"wire_protection":false} to "/api/v1/cluster"
    Then the response status should be 200
    When I PATCH JSON {"forward_foreign_master":false} to "/api/v1/cluster"
    Then the response status should be 200
    When I PATCH JSON {"wire_same_as":1} to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON boolean field "wire_protection" should be false
    When I PATCH JSON {"forward_foreign_master":true} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON field "message" should be "wire_protection_required"

  @id:CLU-011
  Scenario: A refused PATCH changes nothing it named
    When I PATCH JSON {"port":47900,"node_number":8} to "/api/v1/cluster"
    Then the response status should be 422
    When I PATCH JSON {"port":47900,"enabled":true} to "/api/v1/cluster"
    Then the response status should be 409
    When I send a GET request to "/api/v1/cluster"
    Then the JSON numeric field "port" should be 47810
    And the JSON numeric field "node_number" should be 0

  @id:CLU-012
  Scenario: Cluster settings survive a controller restart
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"enabled":true,"cluster_name":"dali-cluster","node_number":2,"wire_same_as":5,"forward_hcl":true,"share_input_events":false} to "/api/v1/cluster"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be true
    And the JSON field "cluster_name" should be "dali-cluster"
    And the JSON numeric field "node_number" should be 2
    And the JSON pointer "/wire_same_as" should be 5
    And the JSON boolean field "forward_hcl" should be true
    And the JSON boolean field "share_input_events" should be false

  @id:CLU-013
  Scenario: A standby reads the cluster settings and refuses to write them
    Given this controller has stood down
    When I send a GET request to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON boolean field "read_only" should be true
    When I PATCH JSON {"cluster_name":"dali-cluster"} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON error should be "controller_standby"
    And the response header "X-Dali2rust-Role" should be "standby"
    When I send a GET request to "/api/v1/cluster"
    Then the JSON field "cluster_name" should be ""

  @id:CLU-014
  Scenario: The cluster settings travel with the configuration a standby pulls
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"cluster_name":"dali-cluster","node_number":4} to "/api/v1/cluster"
    Then the response status should be 200
    When I send a GET request to "/api/v1/config/slices"
    Then the response body should contain "cluster_settings"
    When I export "/api/v1/config/slices/cluster_settings" once it is stored and keep the body
    And I PATCH JSON {"cluster_name":"other","node_number":6} to "/api/v1/cluster"
    Then the response status should be 200
    When I PUT the kept body to "/api/v1/config/slices/cluster_settings"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the JSON pointer "/cluster_name" at "/api/v1/cluster" should eventually be "dali-cluster"
    And the JSON pointer "/node_number" at "/api/v1/cluster" should eventually be "4"
