@stage-I6
Feature: Cluster — the lamps of every node and the state of a channel

  A running node announces the state of its lamps and channel parts, and the
  description of its lamps, in sets of one or more pages per adapter, after a
  change, coalesced, and on a periodic refresh. It keeps a volatile cache of
  what its neighbours announce: a set is shown only when it arrived whole, a
  new board on a node number replaces the old board's lamps at once, and a
  node that falls silent keeps its lamps listed as stale with their state
  unknown. A channel's state is derived on read from its parts on every node.
  The scenarios play the neighbours on the cluster hub in frames of the
  production codec.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"

  @id:CLU-200
  Scenario: A neighbour's lamp state and description appear among the cluster's lamps
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"owner":"site-b"},{"lamp":1,"name":"Hall"}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"lamp":1,"power":"on","level":180}]
    Then the JSON pointer "/state/level" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "180"
    When I send a GET request to "/api/v1/cluster/nodes/1/lamps/1"
    Then the response status should be 200
    And the JSON pointer "/source" should be "cached"
    And the JSON pointer "/name" should be "Hall"
    And the JSON pointer "/state/power" should be "on"
    And the JSON pointer "/cluster_address/node" should be 1
    And the JSON pointer "/home_assistant/unique_id" should be "site-b_a0_vl_1"
    When I send a GET request to "/api/v1/cluster/nodes/1/lamps"
    Then the JSON pointer "/lamps/0/name" should be "Hall"
    And the JSON pointer "/lamps/1" should be absent

  @id:CLU-201
  Scenario: A paged lamp set shows only once its last page has arrived, and the set before it holds meanwhile
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"lamp":1,"name":"One"},{"lamp":2,"name":"Two"},{"lamp":9,"name":"Nine"}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"lamp":9,"power":"off","level":0}]
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/lamps/9" should eventually be "cached"
    When the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 4 as a page before the last with [{"lamp":1,"power":"on","level":90}]
    And the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 2 as the last page with [{"lamp":1,"name":"Probe"},{"lamp":2,"name":"Two"},{"lamp":9,"name":"Nine"}]
    Then the JSON pointer "/name" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "Probe"
    When I send a GET request to "/api/v1/cluster/nodes/1/lamps/1"
    Then the JSON pointer "/state/level" should be null
    When I send a GET request to "/api/v1/cluster/nodes/1/lamps/9"
    Then the JSON pointer "/state/power" should be "off"
    When the board "02:00:00:0a:0b:02" sends lamp state page 1 of set 4 as the last page with [{"lamp":2,"power":"on","level":40}]
    Then the JSON pointer "/state/level" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "90"
    And the JSON pointer "/state/level" at "/api/v1/cluster/nodes/1/lamps/2" should eventually be "40"
    When I send a GET request to "/api/v1/cluster/nodes/1/lamps/9"
    Then the JSON pointer "/state/power" should be "unknown"
    And the JSON pointer "/state/level" should be null

  @id:CLU-202
  Scenario: A neighbour's lamp of unknown level is shown unknown, never zero or off
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"lamp":3,"name":"Porch"}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"lamp":3,"power":"unknown"}]
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/lamps/3" should eventually be "cached"
    When I send a GET request to "/api/v1/cluster/nodes/1/lamps/3"
    Then the JSON pointer "/state/power" should be "unknown"
    And the JSON pointer "/state/level" should be null
    And the JSON pointer "/observation_age_ms" should be null

  @id:CLU-203
  Scenario: A silent node's lamps stay listed as stale with their state unknown
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"lamp":1,"name":"Hall"}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"lamp":1,"power":"on","level":180}]
    Then the JSON pointer "/state/level" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "180"
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "stale"
    When I send a GET request to "/api/v1/cluster/nodes/1/lamps/1"
    Then the JSON pointer "/state/power" should be "unknown"
    And the JSON pointer "/state/level" should be null
    And the JSON pointer "/observation_age_ms" should be null
    When I send a GET request to "/api/v1/cluster/lamps"
    Then the JSON pointer "/nodes/1/source" should be "stale"
    And the JSON pointer "/nodes/1/lamps" should be 1

  @id:CLU-204
  Scenario: A new board on a node number replaces the old board's lamps before its own set is whole
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"lamp":1,"name":"Hall"}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 9 as the last page with [{"lamp":1,"power":"on","level":180}]
    Then the JSON pointer "/state/level" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "180"
    When the board "02:00:00:0a:0b:02" stops pulsing
    And a board "02:00:00:0a:0b:05" at "10.0.0.5" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/board" at "/api/v1/cluster/status" should eventually be "dali-0a0b05"
    When the board "02:00:00:0a:0b:05" sends lamp state page 0 of set 1 as a page before the last with [{"lamp":4,"power":"on","level":20}]
    Then the JSON pointer "/lamps/0" at "/api/v1/cluster/nodes/1/lamps" should eventually be absent
    When the board "02:00:00:0a:0b:05" sends lamp state page 1 of set 1 as the last page with [{"lamp":5,"power":"off","level":0}]
    And the board "02:00:00:0a:0b:05" sends lamp info page 0 of set 1 as the last page with [{"lamp":4,"name":"Four"},{"lamp":5,"name":"Five"}]
    Then the JSON pointer "/lamps/1/cluster_address/lamp" at "/api/v1/cluster/nodes/1/lamps" should eventually be "5"
    And the JSON pointer "/source" at "/api/v1/cluster/nodes/1/lamps/4" should eventually be "cached"

  @id:CLU-205
  Scenario: Forget drops a silent node's lamps
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"lamp":1,"name":"Hall"}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"lamp":1,"power":"on","level":180}]
    Then the JSON pointer "/state/level" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "180"
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/source" at "/api/v1/cluster/nodes/1/lamps/1" should eventually be "stale"
    When I send a DELETE request to "/api/v1/cluster/nodes/1"
    Then the response status should be 204
    When I send a GET request to "/api/v1/cluster/nodes/1/lamps"
    Then the response status should be 404

  @id:CLU-206
  Scenario: This node announces its own lamp after each command, never two sets closer than its coalescing window
    Given adapter 0 has discovered physical devices 0 and 1
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":120} to "/api/v1/adapters/0/virtual-lamps/1/target-state"
    And I PUT JSON {"power":"on","level":140} to "/api/v1/adapters/0/virtual-lamps/1/target-state"
    And I PUT JSON {"power":"on","level":160} to "/api/v1/adapters/0/virtual-lamps/1/target-state"
    And I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/virtual-lamps/1/target-state"
    Then the cluster hub should carry lamp state pages from this controller no two within 20 ms of each other, the last showing lamp 1 at level 180, within 1500 ms

  @id:CLU-207
  Scenario: Renaming a lamp sends its description again
    Given adapter 0 has discovered physical devices 0 and 1
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And I PATCH JSON {"name":"Kitchen"} to "/api/v1/adapters/0/virtual-lamps/1"
    Then the response status should be 200
    And the cluster hub should carry a lamp info page from this controller naming lamp 1 "Kitchen" within 1500 ms

  @id:CLU-208
  Scenario: A broadcast reaches the cluster as one lamp state datagram for the adapter, its commits coalesced
    Given the controller clock reads 600 minutes past midnight
    And adapter 0 has discovered physical devices 0 and 1
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    And I PUT JSON {"physical_short_address":1} to "/api/v1/adapters/0/virtual-lamps/2/binding"
    And I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And I POST JSON {"schedule_id":"house","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"broadcast"}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"absolute","level":80,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the scheduler should drive broadcast to level 80
    And the cluster hub should carry exactly one lamp state page from this controller showing a lamp at level 80, and it shows lamps [1,2] at that level, within 2000 ms

  @id:CLU-209
  Scenario: A channel counts what it knows of its members and shows a silent node's members as unknown, never off
    Given adapter 0 has discovered physical devices 0 and 1
    And physical device 0 reports membership of group 7 read from the gear
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    And I PUT JSON {"power":"off"} to "/api/v1/adapters/0/virtual-lamps/1/target-state"
    Then the JSON pointer "/state/power" at "/api/v1/adapters/0/virtual-lamps/1" should eventually be "off"
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"},{"channel_part":[0,5],"record":16,"mark":[1,5],"send":true,"accept":true}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"part":5,"channel":16,"mark":[1,5],"commanded":true,"lit":1,"known":2,"members":2}]
    Then the JSON pointer "/state/on" at "/api/v1/cluster/channels/16" should eventually be "true"
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the JSON pointer "/state/lit" should be 1
    And the JSON pointer "/state/known" should be 3
    And the JSON pointer "/state/members" should be 3
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/state/known" at "/api/v1/cluster/channels/16" should eventually be "1"
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the JSON pointer "/state/on" should be null
    And the JSON pointer "/state/members" should be 3
    And the JSON pointer "/state/parts_unknown" should be 1

  @id:CLU-210
  Scenario: The cluster channel pushes the lamps summary when a neighbour's lamps move
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "cluster"
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends lamp info page 0 of set 1 as the last page with [{"lamp":1,"name":"Hall"}]
    And the board "02:00:00:0a:0b:02" sends lamp state page 0 of set 1 as the last page with [{"lamp":1,"power":"on","level":180}]
    Then the WebSocket client should receive a cluster lamps frame showing node 1 as "cached" with 1 lamp

  @id:CLU-211
  Scenario: Pointing a channel part at another channel sends the lamp state again at once
    Given adapter 0 has discovered physical devices 0 and 1
    And physical device 0 reports membership of group 7 read from the gear
    When I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the cluster hub should carry a lamp state page from this controller showing part 7 of channel 16 within 5000 ms
    When I PUT JSON {"record":{"number":17,"birth_mark":{"generation":1,"nonce":6}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    Then the cluster hub should carry a lamp state page from this controller showing part 7 of channel 17 within 1000 ms
