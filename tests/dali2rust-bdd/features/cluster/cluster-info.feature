@stage-I6
Feature: Cluster — the channels and scenes of every node

  A running node announces the records it is home to and its own parts in
  pages of one set number, after a change and on a periodic refresh, and keeps
  a volatile cache of what its neighbours announce. A set is shown only when it
  arrived whole; a newer set replaces an older one, and a lost page leaves the
  previous whole set in place. Only a board presence takes as a member of this
  cluster feeds the cache, and a record is the sender's own only when its
  number lies in the sender's block. The cluster's lists merge this node's
  records from the registry with the cached records of the others, each
  marked as own, cached or stale. The scenarios play the neighbours on the
  cluster hub in frames of the production codec.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"

  @id:CLU-057
  Scenario: A neighbour's whole set appears in the cluster's lists beside this node's own records
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/block" at "/api/v1/cluster/status" should eventually be "settled"
    When I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 7 as a page before the last with [{"channel":16,"mark":[1,5],"name":"Hall"},{"scene":17,"mark":[1,6],"name":"Evening"}]
    And the board "02:00:00:0a:0b:02" sends info page 1 of set 7 as the last page with [{"channel_part":[0,3],"record":16,"mark":[1,5],"send":true,"accept":true}]
    Then the JSON pointer "/nodes/1/source" at "/api/v1/cluster/info" should eventually be "cached"
    And the JSON pointer "/cluster/info_sets_taken_total" at "/api/v1/stats" should eventually be greater than 0
    When I send a GET request to "/api/v1/cluster/info"
    Then the response status should be 200
    And the JSON pointer "/nodes/0/source" should be "own"
    And the JSON pointer "/nodes/0/channels" should be 1
    And the JSON pointer "/nodes/1/node" should be 1
    And the JSON pointer "/nodes/1/set" should be 7
    And the JSON pointer "/nodes/1/channels" should be 1
    And the JSON pointer "/nodes/1/scenes" should be 1
    And the JSON pointer "/nodes/1/channel_parts" should be 1
    When I send a GET request to "/api/v1/cluster/channels"
    Then the response status should be 200
    And the JSON pointer "/channels/0/number" should be 2
    And the JSON pointer "/channels/0/source" should be "own"
    And the JSON pointer "/channels/1/number" should be 16
    And the JSON pointer "/channels/1/home_node" should be 1
    And the JSON pointer "/channels/1/name" should be "Hall"
    And the JSON pointer "/channels/1/birth_mark/nonce" should be 5
    And the JSON pointer "/channels/1/source" should be "cached"
    And the JSON pointer "/channels/2" should be absent
    When I send a GET request to "/api/v1/cluster/scenes"
    Then the JSON pointer "/scenes/0/name" should be "Evening"
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the response status should be 200
    And the JSON pointer "/name" should be "Hall"
    And the JSON pointer "/parts/0/node" should be 1
    And the JSON pointer "/parts/0/source" should be "cached"
    And the JSON pointer "/parts/0/group" should be 3
    And the JSON pointer "/parts/0/send" should be true
    And the JSON pointer "/parts/0/link" should be "live"
    When I send a GET request to "/api/v1/cluster/nodes/1/channel-parts"
    Then the response status should be 200
    And the JSON pointer "/node" should be 1
    And the JSON pointer "/channel_parts/0/record/number" should be 16
    And the JSON pointer "/channel_parts/0/link" should be "live"

  @id:CLU-058
  Scenario: A set whose pages did not all arrive is never shown
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as a page before the last with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    And the board "02:00:00:0a:0b:02" sends info page 2 of set 1 as the last page with []
    Then the JSON pointer "/cluster/info_sets_abandoned_total" at "/api/v1/stats" should eventually be greater than 0
    When I send a GET request to "/api/v1/cluster/info"
    Then the JSON pointer "/nodes/1" should be absent
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the response status should be 404

  @id:CLU-059
  Scenario: A lost page leaves the previous whole set in place
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/channels/0/name" at "/api/v1/cluster/channels" should eventually be "Hall"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 2 as a page before the last with [{"channel":16,"mark":[1,5],"name":"Lobby"}]
    And the board "02:00:00:0a:0b:02" sends info page 2 of set 2 as the last page with []
    Then the JSON pointer "/cluster/info_sets_abandoned_total" at "/api/v1/stats" should eventually be greater than 0
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the response status should be 200
    And the JSON pointer "/name" should be "Hall"
    When I send a GET request to "/api/v1/cluster/info"
    Then the JSON pointer "/nodes/1/set" should be 1

  @id:CLU-060
  Scenario: A newer set replaces the shown one and a page of an older set is stale
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    And a board "02:00:00:0a:0b:03" at "10.0.0.3" pulses as node 2 of cluster "hall" for installation "site-c"
    Then the JSON pointer "/nodes/1/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 2 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/channels/0/name" at "/api/v1/cluster/channels" should eventually be "Hall"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 3 as the last page with [{"channel":16,"mark":[1,5],"name":"Lobby"}]
    Then the JSON pointer "/channels/0/name" at "/api/v1/cluster/channels" should eventually be "Lobby"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 2 as the last page with [{"channel":16,"mark":[1,5],"name":"Attic"}]
    And the board "02:00:00:0a:0b:03" sends info page 0 of set 1 as the last page with [{"channel":32,"mark":[1,5],"name":"Stairs"}]
    Then the JSON pointer "/channels/1/name" at "/api/v1/cluster/channels" should eventually be "Stairs"
    And the JSON pointer "/cluster/info_pages_stale_total" at "/api/v1/stats" should eventually be greater than 0
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the JSON pointer "/name" should be "Lobby"
    When I send a GET request to "/api/v1/cluster/info"
    Then the JSON pointer "/nodes/1/set" should be 3

  @id:CLU-061
  Scenario: An empty set clears what the node announced and keeps the node listed
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"},{"scene":17,"mark":[1,6],"name":"Evening"}]
    Then the JSON pointer "/channels/0/name" at "/api/v1/cluster/channels" should eventually be "Hall"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 2 as the last page with []
    Then the JSON pointer "/channels" at "/api/v1/cluster/channels" should eventually be "[]"
    When I send a GET request to "/api/v1/cluster/scenes"
    Then the JSON pointer "/scenes/0" should be absent
    When I send a GET request to "/api/v1/cluster/info"
    Then the JSON pointer "/nodes/1/source" should be "cached"
    And the JSON pointer "/nodes/1/set" should be 2
    And the JSON pointer "/nodes/1/channels" should be 0

  @id:CLU-062
  Scenario: A board presence ignores and a board of another cluster feed nothing
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When a board "02:00:00:0a:0b:03" at "10.0.0.3" pulses as node 1 of cluster "hall" for installation "site-c"
    And a board "02:00:00:0a:0b:04" at "10.0.0.4" pulses as node 2 of cluster "garage" for installation "site-d"
    Then the JSON pointer "/ignored/1/board" at "/api/v1/cluster/status" should eventually contain "dali-0a0b0"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":17,"mark":[1,5],"name":"Real"}]
    Then the JSON pointer "/channels/0/name" at "/api/v1/cluster/channels" should eventually be "Real"
    When the board "02:00:00:0a:0b:03" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Fake"}]
    And the board "02:00:00:0a:0b:04" sends info page 0 of set 1 as the last page with [{"channel":32,"mark":[1,5],"name":"Garage"}]
    Then the JSON pointer "/cluster/info_pages_ignored_total" at "/api/v1/stats" should eventually be greater than 1
    When I send a GET request to "/api/v1/cluster/channels"
    Then the JSON pointer "/channels/0/name" should be "Real"
    And the JSON pointer "/channels/1" should be absent
    When I send a GET request to "/api/v1/cluster/info"
    Then the JSON pointer "/nodes/2" should be absent

  @id:CLU-063
  Scenario: A record outside the sender's block is skipped and proves no channel
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":40,"mark":[1,5],"name":"Copy"},{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/nodes/1/source" at "/api/v1/cluster/info" should eventually be "cached"
    When I send a GET request to "/api/v1/cluster/info"
    Then the JSON pointer "/nodes/1/channels" should be 1
    And the JSON pointer "/nodes/1/skipped" should be 1
    When I send a GET request to "/api/v1/cluster/channels/40"
    Then the response status should be 404

  @id:CLU-064
  Scenario: Lamp maps, group maps and names belong to later slices and are counted and skipped
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"},{"group_map":0},{"group_name":[0,3],"name":"Hall lamps"},{"scene_name":[0,2],"name":"Dim"},{"channel_part":[0,3],"record":16,"mark":[1,5],"send":true,"accept":true,"lamps":1}]
    Then the JSON pointer "/nodes/1/source" at "/api/v1/cluster/info" should eventually be "cached"
    When I send a GET request to "/api/v1/cluster/info"
    Then the JSON pointer "/nodes/1/channels" should be 1
    And the JSON pointer "/nodes/1/channel_parts" should be 1
    And the JSON pointer "/nodes/1/skipped" should be 4
    And the JSON pointer "/cluster/info_elements_skipped_total" at "/api/v1/stats" should eventually be "4"

  @id:CLU-065
  Scenario: This node's records and parts go out in info pages after each write
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/block" at "/api/v1/cluster/status" should eventually be "settled"
    Then the cluster hub should carry a pulse from this controller holding node 0 of cluster "hall"
    When I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    Then the response status should be 201
    And the cluster hub should carry an info page from this controller with {"channel":2,"name":"Corridor"} within 1000 ms
    And the JSON pointer "/cluster/info_sets_sent_total" at "/api/v1/stats" should eventually be greater than 0
    When I link "/api/v1/cluster/channel-parts/0/3" to the cluster record "/api/v1/cluster/channels/2" with send true and accept false
    Then the response status should be 200
    And the cluster hub should carry an info page from this controller with {"channel_part":[0,3],"record":2,"send":true,"accept":false} within 1000 ms
    When I PATCH JSON {"name":"Hall","revision":1} to "/api/v1/cluster/channels/2"
    Then the response status should be 200
    And the cluster hub should carry an info page from this controller with {"channel":2,"name":"Hall"} within 1000 ms

  @id:CLU-066
  Scenario: This node's info goes out again on the periodic refresh with nothing changed
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/block" at "/api/v1/cluster/status" should eventually be "settled"
    When I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    Then the cluster hub should carry an info page from this controller with {"channel":2,"name":"Corridor"} within 4000 ms
    When the cluster hub sets aside what this controller has sent so far
    Then the cluster hub should carry an info page from this controller with {"channel":2,"name":"Corridor"} within 8000 ms

  @id:CLU-067
  Scenario: A node that is off announces nothing
    When I PATCH JSON {"cluster_name":"hall","node_number":0} to "/api/v1/cluster"
    And I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    Then the response status should be 201
    And the cluster hub should carry no datagram from this controller within 1000 ms

  @id:CLU-068
  Scenario: A standby node keeps listening and announces nothing
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/block" at "/api/v1/cluster/status" should eventually be "settled"
    When I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    Then the cluster hub should carry an info page from this controller with {"channel":2,"name":"Corridor"} within 4000 ms
    When I PATCH JSON {"application_active":false} to "/api/v1/settings/dali"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "standby"
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/nodes/1/source" at "/api/v1/cluster/info" should eventually be "cached"
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" says hello
    Then the cluster hub should carry no info page from this controller within 1500 ms

  @id:CLU-069
  Scenario: A node waiting for its number announces nothing and issues no number
    When a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 2 of cluster "hall" for installation "site-b"
    And I PATCH JSON {"cluster_name":"hall","node_number":2,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "waiting"
    And the JSON pointer "/own/block" at "/api/v1/cluster/status" should eventually be "contested"
    When I POST JSON {"number":34,"name":"Corridor"} to "/api/v1/cluster/channels"
    Then the response status should be 409
    And the JSON pointer "/message" should be "cluster_node_contested"
    And the cluster hub should carry no info page from this controller within 1500 ms

  @id:CLU-070
  Scenario: A neighbour's hello is answered with this node's info before the refresh is due
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/block" at "/api/v1/cluster/status" should eventually be "settled"
    When I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the cluster hub should carry an info page from this controller with {"channel":2,"name":"Corridor"} within 4000 ms
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" says hello
    Then the cluster hub should carry an info page from this controller with {"channel":2,"name":"Corridor"} within 1000 ms

  @id:CLU-071
  Scenario: A silent node's info stays as stale until Forget drops it
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/channels/0/source" at "/api/v1/cluster/channels" should eventually be "cached"
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/channels/0/source" at "/api/v1/cluster/channels" should eventually be "stale"
    And the JSON pointer "/nodes/1/source" at "/api/v1/cluster/info" should eventually be "stale"
    When I send a DELETE request to "/api/v1/cluster/nodes/1"
    Then the response status should be 204
    When I send a GET request to "/api/v1/cluster/info"
    Then the JSON pointer "/nodes/1" should be absent
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the response status should be 404

  @id:CLU-072
  Scenario: A part naming an older mark than its settled home node carries is shown as an orphan
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    And a board "02:00:00:0a:0b:03" at "10.0.0.3" pulses as node 2 of cluster "hall" for installation "site-c"
    Then the JSON pointer "/nodes/1/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:03" sends info page 0 of set 1 as the last page with [{"channel":32,"mark":[2,9],"name":"Stairs"}]
    And the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel_part":[0,3],"record":32,"mark":[1,1],"send":true,"accept":true},{"channel_part":[0,4],"record":48,"mark":[1,1],"send":true,"accept":false}]
    And I PUT JSON {"record":{"number":32,"birth_mark":{"generation":2,"nonce":9}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/1/5"
    Then the response status should be 200
    And the JSON pointer "/nodes/2/source" at "/api/v1/cluster/info" should eventually be "cached"
    And the JSON pointer "/nodes/1/source" at "/api/v1/cluster/info" should eventually be "cached"
    When I send a GET request to "/api/v1/cluster/nodes/1/channel-parts"
    Then the response status should be 200
    And the JSON pointer "/channel_parts/0/record/number" should be 32
    And the JSON pointer "/channel_parts/0/link" should be "unknown"
    When the board "02:00:00:0a:0b:03" announces one-page sets from 2 with [{"channel":32,"mark":[2,9],"name":"Stairs"}] until the JSON pointer "/channel_parts/0/link" at "/api/v1/cluster/nodes/1/channel-parts" is "orphan"
    And I send a GET request to "/api/v1/cluster/nodes/1/channel-parts"
    Then the JSON pointer "/channel_parts/0/link" should be "orphan"
    And the JSON pointer "/channel_parts/1/record/number" should be 48
    And the JSON pointer "/channel_parts/1/link" should be "unknown"
    When I send a GET request to "/api/v1/cluster/channels/32"
    Then the response status should be 200
    And the JSON pointer "/name" should be "Stairs"
    And the JSON pointer "/parts/0/node" should be 0
    And the JSON pointer "/parts/0/source" should be "own"
    And the JSON pointer "/parts/0/link" should be "live"
    And the JSON pointer "/parts/1/node" should be 1
    And the JSON pointer "/parts/1/link" should be "orphan"
    When I send a GET request to "/api/v1/cluster/nodes/5/channel-parts"
    Then the response status should be 404
    When I send a GET request to "/api/v1/cluster/nodes/8/scene-parts"
    Then the response status should be 400

  @id:CLU-073
  Scenario: The cluster channel pushes the info resource when a neighbour's set arrives
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "cluster"
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the WebSocket client should receive a cluster info frame showing node 1 as "cached" with 1 channel

  @id:CLU-074
  Scenario: Moving to another cluster drops what the neighbours of the old one announced
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/nodes/1/source" at "/api/v1/cluster/info" should eventually be "cached"
    When I PATCH JSON {"cluster_name":"yard","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/nodes/1" at "/api/v1/cluster/info" should eventually be absent
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the response status should be 404

  @id:CLU-075
  Scenario: Switching the cluster off drops what the neighbours announced
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/nodes/1/source" at "/api/v1/cluster/info" should eventually be "cached"
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":false} to "/api/v1/cluster"
    Then the JSON pointer "/nodes/1" at "/api/v1/cluster/info" should eventually be absent
