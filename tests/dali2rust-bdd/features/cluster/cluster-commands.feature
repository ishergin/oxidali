@stage-I6
Feature: Cluster — commands to channels and cluster scenes between nodes

  A command to a channel, or a recall of a cluster scene, given on one node
  goes to the network as one datagram sent twice with one sequence number, and
  each node that holds an accepting part of the record executes it on its own
  DALI group or scene. A node takes a command only from a board it takes as a
  member of its cluster, once, never its own, and only while it runs; on a
  shared wire only the wire's owner executes, and a command already executed on
  this node's wire is executed by nobody here. The node that sends a command
  executes its own part itself. The scenarios play the neighbours on the
  cluster hub in frames of the production codec.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"

  @id:CLU-076
  Scenario: A neighbour's command to a channel is executed once on the linked group and never sent back
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    Then the response status should be 200
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":200} twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_duplicate_total" at "/api/v1/stats" should eventually be "1"
    And the DALI mock transport forward trace should eventually be exactly "8EC8"
    And virtual lamp 1 on adapter 0 should eventually report level 200 from value_source "cluster"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/forwards_skipped_cluster_origin_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms

  @id:CLU-077
  Scenario Outline: A command for a part this node does not hold, or will not take, or from no member, executes nothing
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":<accept>} to "/api/v1/cluster/channel-parts/0/7"
    Then the response status should be 200
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "<cluster>" for installation "site-b"
    Then the JSON pointer "<seen>" at "/api/v1/cluster/status" should eventually be "<as>"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,<nonce>],"level":200} twice with one sequence number
    Then the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the JSON pointer "/cluster/<dropped>" at "/api/v1/stats" should eventually be "<times>"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "0"
    And the DALI mock transport should have received 0 forward frames

    Examples:
      | cluster | seen              | as            | accept | nonce | dropped                           | times |
      | hall    | /nodes/0/heard    | true          | true   | 6     | commands_dropped_no_part_total    | 1     |
      | hall    | /nodes/0/heard    | true          | false  | 5     | commands_dropped_accept_off_total | 1     |
      | garage  | /ignored/0/reason | other_cluster | true   | 5     | commands_dropped_not_member_total | 2     |

  @id:CLU-078
  Scenario: A command carrying this node's own hardware address is never taken
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When a datagram with this controller's own hardware address carries the command {"channel":[16,1,5],"level":200} as node 1 of cluster "hall"
    And the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":100} twice with one sequence number
    Then the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the DALI mock transport forward trace should eventually be exactly "8E64"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"

  @id:CLU-079
  Scenario: A command already executed on this node's wire is skipped here
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":200,"executed_on_wire":0} twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_executed_on_wire_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the DALI mock transport should have received 0 forward frames

  @id:CLU-080
  Scenario: On a shared wire the node that does not own it executes nothing
    When I PATCH JSON {"cluster_name":"hall","node_number":2,"wire_same_as":1,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/own/wire_owner/node" at "/api/v1/cluster/status" should eventually be "1"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":200} twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_not_wire_owner_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the DALI mock transport should have received 0 forward frames

  @id:CLU-088
  Scenario: A command executed on another wire is executed here
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":200,"executed_on_wire":1} twice with one sequence number
    Then the DALI mock transport forward trace should eventually be exactly "8EC8"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_dropped_executed_on_wire_total" at "/api/v1/stats" should eventually be "0"

  @id:CLU-089
  Scenario: Without wire protection a node that shares a wire it does not own executes a command, even one executed on that wire
    When I PATCH JSON {"cluster_name":"hall","node_number":2,"wire_same_as":1,"wire_protection":false,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/own/wire_owner/node" at "/api/v1/cluster/status" should eventually be "1"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":200} twice with one sequence number
    Then the DALI mock transport forward trace should eventually be exactly "8EC8"
    When the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":100,"executed_on_wire":1} twice with one sequence number
    Then the DALI mock transport forward trace should eventually be exactly "8EC8, 8E64"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "2"

  @id:CLU-081
  Scenario: A cluster scene recalled on the whole node is recalled on every accepting scene part
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":17,"birth_mark":{"generation":1,"nonce":6}},"send":true,"accept":true} to "/api/v1/cluster/scene-parts/0/4"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"scene":[17,1,6]} twice with one sequence number
    Then the DALI mock transport forward trace should eventually be exactly "FF14"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 500 ms

  @id:CLU-082
  Scenario: A cluster scene recalled on a channel is recalled on the channel's group
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":17,"birth_mark":{"generation":1,"nonce":6}},"send":true,"accept":true} to "/api/v1/cluster/scene-parts/0/4"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"scene":[17,1,6],"channel":[16,1,5]} twice with one sequence number
    Then the DALI mock transport forward trace should eventually be exactly "8F14"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-087
  Scenario Outline: A cluster scene recalled on a channel executes nothing unless both parts accept
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":17,"birth_mark":{"generation":1,"nonce":6}},"send":true,"accept":<scene_accepts>} to "/api/v1/cluster/scene-parts/0/4"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":<group_accepts>} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"scene":[17,1,6],"channel":[16,1,5]} twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_accept_off_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "0"
    And the DALI mock transport should have received 0 forward frames

    Examples:
      | scene_accepts | group_accepts |
      | false         | true          |
      | true          | false         |

  @id:CLU-083
  Scenario: A channel command given here goes out twice with one sequence number and runs on this node's own part
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":false} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 3 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/nodes/1/source" at "/api/v1/cluster/info" should eventually be "cached"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":200} to "/api/v1/cluster/channels/16/target-state"
    Then the response status should be 200
    And the JSON field "status" should be "sent"
    And the cluster hub should carry from this controller the command {"channel":[16,1,5],"level":200,"executed_on_wire":0} twice with one sequence number
    And the DALI mock transport forward trace should eventually be exactly "8EC8"
    And the JSON pointer "/cluster/commands_sent_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-084
  Scenario: A second setpoint to a channel inside the spacing waits and goes out after the first
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    Then the response status should be 201
    When I link "/api/v1/cluster/channel-parts/0/7" to the cluster record "/api/v1/cluster/channels/2" with send true and accept true
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":10} to "/api/v1/cluster/channels/2/target-state"
    Then the response status should be 200
    When I PUT JSON {"power":"on","level":20} to "/api/v1/cluster/channels/2/target-state"
    Then the response status should be 200
    And the JSON pointer "/cluster/commands_delayed_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_sent_total" at "/api/v1/stats" should eventually be "2"
    And the DALI mock transport forward trace should eventually be exactly "8E0A, 8E14"

  @id:CLU-085
  Scenario: A command is refused while the cluster is off or this node is not running, and for a record nobody knows
    When I PATCH JSON {"cluster_name":"hall","node_number":2} to "/api/v1/cluster"
    Then the response status should be 200
    When I POST JSON {"number":32,"name":"Corridor"} to "/api/v1/cluster/channels"
    Then the response status should be 201
    When I POST JSON {"number":33,"name":"Evening"} to "/api/v1/cluster/scenes"
    Then the response status should be 201
    When I PUT JSON {"power":"on","level":200} to "/api/v1/cluster/channels/32/target-state"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "cluster_off"
    When I PUT JSON {"power":"on","level":200} to "/api/v1/cluster/channels/16/target-state"
    Then the response status should be 404
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 2 of cluster "hall" for installation "site-b"
    And I PATCH JSON {"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "waiting"
    When I send a POST request to "/api/v1/cluster/scenes/33/recall"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_not_running"
    When I PUT JSON {"power":"on","level":200} to "/api/v1/cluster/channels/20/target-state"
    Then the response status should be 404
    When I send a POST request to "/api/v1/cluster/scenes/40/recall"
    Then the response status should be 404
    When I POST JSON {"channel":40} to "/api/v1/cluster/scenes/33/recall"
    Then the response status should be 404
    When I POST JSON {"channel":"hall"} to "/api/v1/cluster/scenes/33/recall"
    Then the response status should be 422
    When I send a GET request to "/api/v1/stats"
    Then the JSON pointer "/cluster/commands_sent_total" should be 0

  @id:CLU-086
  Scenario: A standby executes no neighbour's command and refuses to send one
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And I PATCH JSON {"application_active":false} to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "standby"
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":200} twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_not_running_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the DALI mock transport should have received 0 forward frames
    When I PUT JSON {"power":"on","level":200} to "/api/v1/cluster/channels/2/target-state"
    Then the response status should be 409
    And the JSON error should be "controller_standby"
