@stage-I6
Feature: Cluster — a command to one lamp of another node

  Every virtual lamp of a node is a cluster lamp addressed by node, adapter and
  lamp. A command to it given on another node goes to the network as one
  datagram sent twice with one sequence number; only the node the address
  names executes it, as a command to its own virtual lamp, and only while it
  accepts commands to single lamps. The wire owner rule does not apply: the
  owning node executes its lamp whoever owns the wire, and a node that merely
  shares the wire executes nothing. A command for this node's own lamp goes
  down the virtual lamp's own route. The sender checks the lamp against what
  its node announced; the owner checks again and counts what it refuses. The
  scenarios play the neighbours on the cluster hub in frames of the production
  codec.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"

  @id:CLU-220
  Scenario: A neighbour's command to this node's lamp is executed once on the bound device's short address
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" sends the command {"lamp":[0,0,1],"level":200} twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_duplicate_total" at "/api/v1/stats" should eventually be "1"
    And the DALI mock transport forward trace should eventually be exactly "00C8"
    And virtual lamp 1 on adapter 0 should eventually report level 200 from value_source "cluster"
    And the JSON pointer "/state/value_source" at "/api/v1/cluster/nodes/0/lamps/1" should eventually be "cluster"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms

  @id:CLU-221
  Scenario: A lamp takes a setpoint and a level preset from a neighbour, and a relative verb is counted unsupported
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"lamp":[0,0,1],"dim":"up"} twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_unsupported_total" at "/api/v1/stats" should eventually be "1"
    When the board "02:00:00:0a:0b:02" sends the command {"lamp":[0,0,1],"recall":"max_level"} twice with one sequence number
    Then the DALI mock transport forward trace should eventually be exactly "0105"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_dropped_unsupported_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-222
  Scenario Outline: A lamp command this node will not take, or for a lamp it does not hold, or from no member, executes nothing
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PATCH JSON <settings> to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "<cluster>" for installation "site-b"
    Then the JSON pointer "<seen>" at "/api/v1/cluster/status" should eventually be "<as>"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command <command> twice with one sequence number
    Then the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the JSON pointer "/cluster/<dropped>" at "/api/v1/stats" should eventually be "<times>"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "0"
    And the DALI mock transport should have received 0 forward frames

    Examples:
      | settings                                                                              | cluster | seen              | as            | command                                  | dropped                                | times |
      | {"cluster_name":"hall","node_number":0,"enabled":true}                                | hall    | /nodes/0/heard    | true          | {"lamp":[0,0,1],"level":200}             | lamp_commands_dropped_accept_off_total | 1     |
      | {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true}     | hall    | /nodes/0/heard    | true          | {"lamp":[3,0,1],"level":200}             | commands_dropped_no_part_total         | 1     |
      | {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true}     | hall    | /nodes/0/heard    | true          | {"lamp":[0,0,2],"level":200}             | lamp_commands_refused_total            | 1     |
      | {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true}     | hall    | /nodes/0/heard    | true          | {"lamp":[0,1,1],"level":200}             | lamp_commands_refused_total            | 1     |
      | {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true}     | hall    | /nodes/0/heard    | true          | {"lamp":[0,0,1],"level":200,"rgb":[255,0,0]} | lamp_commands_refused_total        | 1     |
      | {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true}     | garage  | /ignored/0/reason | other_cluster | {"lamp":[0,0,1],"level":200}             | commands_dropped_not_member_total      | 2     |

  @id:CLU-223
  Scenario: A lamp command carrying this node's own hardware address is never taken
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When a datagram with this controller's own hardware address carries the command {"lamp":[0,0,1],"level":200} as node 1 of cluster "hall"
    Then the JSON pointer "/cluster/own_frames_dropped_total" at "/api/v1/stats" should eventually be "1"
    When the board "02:00:00:0a:0b:02" sends the command {"lamp":[0,0,1],"level":100} twice with one sequence number
    Then the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the DALI mock transport forward trace should eventually be exactly "0064"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"

  @id:CLU-224
  Scenario: A standby executes no neighbour's lamp command
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true} to "/api/v1/cluster"
    And I PATCH JSON {"application_active":false} to "/api/v1/settings/dali"
    Then the response status should be 200
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "standby"
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"lamp":[0,0,1],"level":200} twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_not_running_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "0"
    And the DALI mock transport should have received 0 forward frames

  @id:CLU-225
  Scenario: On a shared wire the lamp's own node executes it without owning the wire, and the node it does not name executes nothing
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PATCH JSON {"cluster_name":"hall","node_number":2,"wire_same_as":1,"enabled":true,"accept_single_lamps":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/own/wire_owner/node" at "/api/v1/cluster/status" should eventually be "1"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"lamp":[1,0,1],"level":200} twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_no_part_total" at "/api/v1/stats" should eventually be "1"
    And the DALI mock transport should have received 0 forward frames
    When the board "02:00:00:0a:0b:02" sends the command {"lamp":[2,0,1],"level":200,"executed_on_wire":1} twice with one sequence number
    Then the DALI mock transport forward trace should eventually be exactly "00C8"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_dropped_not_wire_owner_total" at "/api/v1/stats" should eventually be "0"
    And the JSON pointer "/cluster/commands_dropped_executed_on_wire_total" at "/api/v1/stats" should eventually be "0"

  @id:CLU-226
  Scenario: A lamp command executed here is never sent back, and a command to this node's own lamp never goes out
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    Then the response status should be 200
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" sends the command {"lamp":[0,0,1],"level":200} twice with one sequence number
    Then the DALI mock transport forward trace should eventually be exactly "00C8"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms
    When I PUT JSON {"power":"on","level":120} to "/api/v1/cluster/nodes/0/lamps/1/target-state"
    Then the response status should be 200
    And the JSON pointer "/virtual_lamp_id" should be 1
    And the JSON pointer "/binding/physical_short_address" should be 0
    And the DALI mock transport forward trace should eventually be exactly "00C8, 0078"
    And virtual lamp 1 on adapter 0 should eventually report level 120 from value_source "api"
    And the cluster hub should carry no command from this controller within 1000 ms
    And the JSON pointer "/cluster/commands_sent_total" at "/api/v1/stats" should eventually be "0"

  @id:CLU-227
  Scenario: A command to another node's lamp goes out twice with one sequence number and is executed by nobody here
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"accept_single_lamps":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"lamp":1,"name":"Hall"}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"lamp":1,"power":"off","level":0}]
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "cached"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":120} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 200
    And the JSON field "status" should be "sent"
    And the cluster hub should carry from this controller the command {"lamp":[1,0,1],"level":120} twice with one sequence number
    When I PUT JSON {"recall":"min_level"} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 200
    And the cluster hub should carry from this controller the command {"lamp":[1,0,1],"recall":"min_level"} twice with one sequence number
    And the JSON pointer "/cluster/commands_sent_total" at "/api/v1/stats" should eventually be "2"
    And the DALI mock transport should have received 0 forward frames

  @id:CLU-228
  Scenario: A command to another node's lamp is checked against what that node announced before anything is sent
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"lamp":1,"name":"Hall","cct":[2700,6500]},{"lamp":2,"name":"Spare","bound":false}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"lamp":1,"power":"off","level":0},{"lamp":2,"power":"unknown"}]
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/lamps/2" should eventually be "cached"
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":120} to "/api/v1/cluster/nodes/3/lamps/1/target-state"
    Then the response status should be 404
    When I PUT JSON {"power":"on","level":120} to "/api/v1/cluster/nodes/1/lamps/9/target-state"
    Then the response status should be 404
    When I PUT JSON {"power":"on","level":120} to "/api/v1/cluster/nodes/1/lamps/2/target-state"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "cluster_lamp_unbound"
    When I PUT JSON {"color_mode":"xy","xy":{"x":0.3,"y":0.3}} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 422
    And the JSON error should be "unsupported_capability"
    When I PUT JSON {"dim":"up"} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"
    When I PUT JSON {"power":"on","color_temperature_kelvin":3000} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 200
    And the cluster hub should carry from this controller the command {"lamp":[1,0,1],"power":"on","kelvin":3000} twice with one sequence number
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "stale"
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":120} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_lamp_stale"
    When I PUT JSON {"color_mode":"xy","xy":{"x":0.3,"y":0.3}} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_lamp_stale"
    And the cluster hub should carry no command from this controller within 500 ms
    And the JSON pointer "/cluster/commands_sent_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-229
  Scenario: A command to another node's lamp is refused while the cluster is off or this node is not running
    When I PATCH JSON {"cluster_name":"hall","node_number":2} to "/api/v1/cluster"
    Then the response status should be 200
    When I PUT JSON {"power":"on","level":200} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "cluster_off"
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 2 of cluster "hall" for installation "site-b"
    And I PATCH JSON {"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "waiting"
    When I PUT JSON {"power":"on","level":200} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_not_running"
    When I PATCH JSON {"application_active":false} to "/api/v1/settings/dali"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "standby"
    When I PUT JSON {"power":"on","level":200} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 409
    And the JSON error should be "controller_standby"
    When I send a GET request to "/api/v1/stats"
    Then the JSON pointer "/cluster/commands_sent_total" should be 0

  @id:CLU-230
  Scenario: A slider on another node's lamp leaves every value in its order, delayed by the target spacing, the last one last
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"lamp":1,"name":"Hall"}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"lamp":1,"power":"off","level":0}]
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "cached"
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":10} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    And I PUT JSON {"power":"on","level":20} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    And I PUT JSON {"power":"on","level":30} to "/api/v1/cluster/nodes/1/lamps/1/target-state"
    Then the response status should be 200
    And the cluster hub should carry from this controller the commands [{"lamp":[1,0,1],"level":10},{"lamp":[1,0,1],"level":20},{"lamp":[1,0,1],"level":30}] in this order, each twice with one sequence number
    And the JSON pointer "/cluster/commands_delayed_total" at "/api/v1/stats" should eventually be "2"
    And the JSON pointer "/cluster/commands_sent_total" at "/api/v1/stats" should eventually be "3"
