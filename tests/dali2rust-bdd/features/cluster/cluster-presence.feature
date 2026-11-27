@stage-I6
Feature: Cluster — presence of the nodes

  A switched-on node pulses on the cluster's broadcast port, listens two pulse
  periods before it holds its number, and keeps a table of the boards it hears:
  the nodes of its own cluster, and the boards it ignores with the reason. A
  node it no longer hears is forgotten on request, never while it is heard. The
  cluster hub stands for the network: the scenarios read what this controller
  sent on it and play the neighbours on it in production frames.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"

  @id:CLU-015
  Scenario: A cluster that is off sends nothing, and once on it holds its number in a pulse
    When I PATCH JSON {"cluster_name":"hall","node_number":2} to "/api/v1/cluster"
    Then the response status should be 200
    And the cluster hub should carry no datagram from this controller within 1000 ms
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "off"
    When I PATCH JSON {"enabled":true} to "/api/v1/cluster"
    Then the response status should be 200
    And the cluster hub should carry a pulse from this controller holding node 2 of cluster "hall"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    And the JSON pointer "/own/board" at "/api/v1/cluster/status" should eventually be "dali-0a0b01"
    And the JSON pointer "/cluster/datagrams_sent_total" at "/api/v1/stats" should eventually be greater than 0

  @id:CLU-016
  Scenario: A neighbour's pulse makes it a heard node with its board, address and installation
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/nodes/0/node" at "/api/v1/cluster/status" should eventually be "1"
    And the JSON pointer "/nodes/0/board" at "/api/v1/cluster/status" should eventually be "dali-0a0b02"
    And the JSON pointer "/nodes/0/address" at "/api/v1/cluster/status" should eventually be "10.0.0.2"
    And the JSON pointer "/nodes/0/installation" at "/api/v1/cluster/status" should eventually be "site-b"
    And the JSON pointer "/ignored" at "/api/v1/cluster/status" should eventually be "[]"
    And the JSON pointer "/cluster/datagrams_received_total" at "/api/v1/stats" should eventually be greater than 0

  @id:CLU-017
  Scenario: A board of another cluster is ignored, and the reason names that cluster
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "garage" for installation "site-b"
    Then the JSON pointer "/ignored/0/reason" at "/api/v1/cluster/status" should eventually be "other_cluster"
    And the JSON pointer "/ignored/0/cluster_name" at "/api/v1/cluster/status" should eventually be "garage"
    And the JSON pointer "/ignored/0/board" at "/api/v1/cluster/status" should eventually be "dali-0a0b02"
    And the JSON pointer "/nodes" at "/api/v1/cluster/status" should eventually be "[]"
    And the JSON pointer "/cluster/other_cluster_frames_total" at "/api/v1/stats" should eventually be greater than 0

  @id:CLU-018
  Scenario: A number another installation holds keeps this node waiting
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 2 of cluster "hall" for installation "site-b"
    And I PATCH JSON {"cluster_name":"hall","node_number":2,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "waiting"
    And the JSON pointer "/own/number_held_by" at "/api/v1/cluster/status" should eventually be "dali-0a0b02"
    And the cluster hub should carry no pulse from this controller holding node 2 within 1500 ms

  @id:CLU-019
  Scenario: A datagram carrying this controller's own hardware address is never a neighbour
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board at "10.0.0.9" pulses this controller's hardware address as node 1 of cluster "hall"
    Then the JSON pointer "/cluster/own_frames_dropped_total" at "/api/v1/stats" should eventually be greater than 0
    When I send a GET request to "/api/v1/cluster/status"
    Then the response status should be 200
    And the JSON pointer "/nodes/0" should be absent
    And the JSON pointer "/ignored/0" should be absent

  @id:CLU-020
  Scenario: Forget answers 409 while the node is heard and 204 once it is silent
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When I send a DELETE request to "/api/v1/cluster/nodes/1"
    Then the response status should be 409
    And the JSON field "message" should be "node_heard"
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "false"
    When I send a DELETE request to "/api/v1/cluster/nodes/1"
    Then the response status should be 204
    And the JSON pointer "/nodes" at "/api/v1/cluster/status" should eventually be "[]"

  @id:CLU-021
  Scenario: Forget refuses a node number outside the cluster
    When I send a DELETE request to "/api/v1/cluster/nodes/8"
    Then the response status should be 400
    And the JSON field "error" should be "invalid_resource_id"

  @id:CLU-022
  Scenario: The cluster channel pushes the status resource when a node is heard
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "cluster"
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the WebSocket client should receive a cluster status frame showing node 1 heard from board "dali-0a0b02"

  @id:CLU-023
  Scenario: A standby board listens to the cluster but never transmits
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the response status should be 200
    And the cluster hub should carry a pulse from this controller holding node 0 of cluster "hall"
    When I PATCH JSON {"application_active":false} to "/api/v1/settings/dali"
    Then the response status should be 200
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "standby"
    And the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/cluster/withheld_standby_total" at "/api/v1/stats" should eventually be greater than 0
    When the cluster hub sets aside what this controller has sent so far
    Then the cluster hub should carry no datagram from this controller within 1000 ms
