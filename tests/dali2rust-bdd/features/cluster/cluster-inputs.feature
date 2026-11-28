@stage-I6
Feature: Cluster — the input devices of every node

  A running node that shares its input events sends each event of its
  IEC 62386-103 input devices that names a short address, twice with one
  sequence number in the line of its commands, never folding two into one.
  Illuminance leaves at most once a second and position once a window, the
  newest value trailing; every occupancy report leaves. It pages its input
  devices like its channels, and a neighbour shows a set only when it arrived
  whole. A neighbour's event is taken once, only from a member of this
  cluster, and becomes one bus event that the WebSocket carries in the local
  shape with the node. The scenarios play the neighbours on the cluster hub in
  frames of the production codec.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"

  @id:CLU-240
  Scenario: A press and its release leave twice each with every field and in their order
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a 24-bit input event frame 06 04 01 is observed on the bus
    And a 24-bit input event frame 06 04 00 is observed on the bus
    Then the cluster hub should carry from this controller the input events [{"adapter":0,"device":3,"instance":null,"instance_type":1,"groups":[null,null,null],"scheme":1,"kind":"button","event_info":1,"value":1},{"adapter":0,"device":3,"instance":null,"instance_type":1,"groups":[null,null,null],"scheme":1,"kind":"button","event_info":0,"value":0}] in this order, each twice with one sequence number
    And the cluster hub should carry no input event from this controller within 500 ms
    And the JSON pointer "/cluster/input_events_sent_total" at "/api/v1/stats" should eventually be "2"

  @id:CLU-241
  Scenario: An event that names no short address stays home and is counted
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a 24-bit input event frame 82 80 01 is observed on the bus
    Then the JSON pointer "/cluster/input_events_unaddressed_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no input event from this controller within 500 ms
    And the JSON pointer "/cluster/input_events_sent_total" at "/api/v1/stats" should eventually be "0"

  @id:CLU-242
  Scenario: A node that does not share its input events sends neither its events nor its input devices
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"share_input_events":false} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a 24-bit input event frame 06 04 01 is observed on the bus
    Then the cluster hub should carry no input event from this controller within 500 ms
    And the cluster hub should carry no input info page from this controller within 500 ms
    And the JSON pointer "/cluster/input_events_sent_total" at "/api/v1/stats" should eventually be "0"

  @id:CLU-243
  Scenario: Illuminance leaves at most once a second and the newest value trails
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a 24-bit input event frame 08 11 2C is observed on the bus
    And a 24-bit input event frame 08 11 36 is observed on the bus
    And a 24-bit input event frame 08 11 40 is observed on the bus
    Then the cluster hub should carry from this controller the input events [{"adapter":0,"device":4,"instance":null,"instance_type":4,"groups":[null,null,null],"scheme":1,"kind":"illuminance","event_info":300,"value":300},{"adapter":0,"device":4,"instance":null,"instance_type":4,"groups":[null,null,null],"scheme":1,"kind":"illuminance","event_info":320,"value":320}] in this order, each twice with one sequence number
    And the cluster hub should carry no input event from this controller within 1500 ms

  @id:CLU-244
  Scenario: Position leaves once a window and the newest value trails
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a 24-bit input event frame 0C 08 0A is observed on the bus
    And a 24-bit input event frame 0C 08 14 is observed on the bus
    And a 24-bit input event frame 0C 08 1E is observed on the bus
    Then the cluster hub should carry from this controller the input events [{"adapter":0,"device":6,"instance":null,"instance_type":2,"groups":[null,null,null],"scheme":1,"kind":"position","event_info":10,"value":10},{"adapter":0,"device":6,"instance":null,"instance_type":2,"groups":[null,null,null],"scheme":1,"kind":"position","event_info":30,"value":30}] in this order, each twice with one sequence number
    And the cluster hub should carry no input event from this controller within 800 ms

  @id:CLU-245
  Scenario: Every occupancy report leaves, the same report twice included
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a 24-bit input event frame 0A 0C 03 is observed on the bus
    And a 24-bit input event frame 0A 0C 03 is observed on the bus
    And a 24-bit input event frame 0A 0C 02 is observed on the bus
    Then the cluster hub should carry from this controller the input events [{"adapter":0,"device":5,"instance":null,"instance_type":3,"groups":[null,null,null],"scheme":1,"kind":"occupancy","event_info":3,"value":3},{"adapter":0,"device":5,"instance":null,"instance_type":3,"groups":[null,null,null],"scheme":1,"kind":"occupancy","event_info":3,"value":3},{"adapter":0,"device":5,"instance":null,"instance_type":3,"groups":[null,null,null],"scheme":1,"kind":"occupancy","event_info":2,"value":2}] in this order, each twice with one sequence number
    And the cluster hub should carry no input event from this controller within 500 ms

  @id:CLU-246
  Scenario: A neighbour's press and its copy become one event on the WebSocket in the local shape with the node
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "input"
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends the input event {"adapter":0,"device":3,"instance":0,"instance_type":1,"groups":[4,null,null],"scheme":2,"kind":"button","event_info":1,"value":0} twice with one sequence number
    Then the WebSocket client should receive a "ClusterInputEventObservedEvent" frame on channel "input" whose payload "node" is 1
    And the JSON pointer "/payload/short_address" of the remembered frame should be 3
    And the JSON pointer "/payload/instance_number" of the remembered frame should be 0
    And the JSON pointer "/payload/instance_group" of the remembered frame should be 4
    And the JSON pointer "/payload/event" of the remembered frame should be "press"
    And the WebSocket client should receive no frame within 600 ms
    And the JSON pointer "/cluster/input_events_received_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/input_events_dropped_total" at "/api/v1/stats" should eventually be "0"

  @id:CLU-247
  Scenario: An input event from another cluster or with this controller's own hardware address does nothing
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "garage" for installation "site-b"
    And the board "02:00:00:0a:0b:02" sends the input event {"adapter":0,"device":3,"instance":0,"instance_type":1,"groups":[null,null,null],"scheme":2,"kind":"button","event_info":1,"value":0} twice with one sequence number
    And a datagram with this controller's own hardware address carries the input event {"adapter":0,"device":3,"instance":0,"instance_type":1,"groups":[null,null,null],"scheme":2,"kind":"button","event_info":1,"value":0} as node 0 of cluster "hall"
    Then the JSON pointer "/cluster/input_events_received_total" at "/api/v1/stats" should stay "0" for 600 ms

  @id:CLU-248
  Scenario: A neighbour's input devices appear only once their set arrived whole
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends input info page 0 of set 5 as a page before the last with [{"owner":["barn","homeassistant"]},{"device":3,"name":"Hall panel","instances":2,"ha_entity":true}]
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/inputs" should stay absent for 400 ms
    When the board "02:00:00:0a:0b:02" sends input info page 1 of set 5 as the last page with [{"instance":[3,0],"instance_type":1,"groups":[4,null,null],"last":[1,null,1500]},{"instance":[3,1],"instance_type":1}]
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/inputs" should eventually be "cached"
    When I send a GET request to "/api/v1/cluster/nodes/1/inputs"
    Then the response status should be 200
    And the JSON pointer "/node" should be 1
    And the JSON pointer "/set" should be 5
    And the JSON pointer "/device_count" should be 1
    And the JSON pointer "/owner/controller_id" should be "barn"
    And the JSON pointer "/devices/0/short_address" should be 3
    And the JSON pointer "/devices/0/name" should be "Hall panel"
    And the JSON pointer "/devices/0/source" should be "cached"
    And the JSON pointer "/devices/0/ha_entity" should be true
    And the JSON pointer "/devices/0/instances/0/instance_type_name" should be "push_button"
    And the JSON pointer "/devices/0/instances/0/instance_groups/0" should be 4
    And the JSON pointer "/devices/0/instances/0/runtime/last_event_info" should be 1
    And the JSON pointer "/devices/0/instances/0/runtime/age_ms" should be greater than 1499
    And the JSON pointer "/devices/0/instances/1/runtime/age_ms" should be null
    When I send a GET request to "/api/v1/cluster/nodes/0/inputs"
    Then the response status should be 200
    And the JSON pointer "/source" should be "own"

  @id:CLU-249
  Scenario: A silent neighbour's input devices go stale and Forget drops them
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends input info page 0 of set 2 as the last page with [{"device":3,"name":"Hall panel","instances":1},{"instance":[3,0],"instance_type":1}]
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/inputs" should eventually be "cached"
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/inputs" should eventually be "stale"
    And the JSON pointer "/devices/0/source" at "/api/v1/cluster/nodes/1/inputs" should eventually be "stale"
    When I send a DELETE request to "/api/v1/cluster/nodes/1"
    Then the response status should be 204
    When I send a GET request to "/api/v1/cluster/nodes/1/inputs"
    Then the response status should be 404

  @id:CLU-250
  Scenario: A standby sends neither its input events nor its input devices
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PATCH JSON {"application_active":false} to "/api/v1/settings/dali"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "standby"
    When the cluster hub sets aside what this controller has sent so far
    And a 24-bit input event frame 06 04 01 is observed on the bus
    Then the cluster hub should carry no input event from this controller within 500 ms
    And the cluster hub should carry no input info page from this controller within 500 ms

  @id:CLU-251
  Scenario: The cluster channel tells the client when a neighbour's input devices change
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "cluster"
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends input info page 0 of set 9 as the last page with [{"device":3,"name":"Hall panel","instances":1},{"instance":[3,0],"instance_type":1}]
    Then the WebSocket client should receive a cluster inputs frame showing node 1 as "cached" with 1 device

  @id:CLU-252
  Scenario: A neighbour's press that arrives after its own release is dropped and counted
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "input"
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends the input events {"adapter":0,"device":3,"instance":0,"instance_type":1,"groups":[null,null,null],"scheme":2,"kind":"button","event_info":1,"value":0} and then {"adapter":0,"device":3,"instance":0,"instance_type":1,"groups":[null,null,null],"scheme":2,"kind":"button","event_info":0,"value":0}, the later one arriving first
    Then the WebSocket client should receive a "ClusterInputEventObservedEvent" frame on channel "input" whose payload "node" is 1
    And the JSON pointer "/payload/event" of the remembered frame should be "release"
    And the WebSocket client should receive no frame within 600 ms
    And the JSON pointer "/cluster/input_events_dropped_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/input_events_received_total" at "/api/v1/stats" should eventually be "1"
