@stage-I6
Feature: Cluster — copies of the records this node's parts name, orphans and wire faults

  A node keeps a copy of each record of another node that one of its parts
  names: number, birth mark, name and revision, taken only from the home
  node's own whole set and only at a higher revision. The copies are a
  persisted slice of their own, so a record whose home is not heard is still
  listed, under the copy's name and as known only by its copy. A part whose
  home node, settled and heard whole, holds no record with its mark is an
  orphan; it is reported, never removed by itself, and removed by deleting
  the part. A part whose neighbour on the same wire holds another record for
  that adapter and group, or none, is reported as a wire fault and changes
  nothing. The scenarios play the neighbours on the cluster hub in frames of
  the production codec.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/block" at "/api/v1/cluster/status" should eventually be "settled"

  @id:CLU-140
  Scenario: A copy is stored only after the home node's whole set and survives a restart
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    Then the JSON pointer "/own/block" at "/api/v1/cluster/status" should eventually be "settled"
    When I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/3"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as a page before the last with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/cluster/copies_stored_total" at "/api/v1/stats" should stay "0" for 1000 ms
    When the board "02:00:00:0a:0b:02" sends info page 1 of set 1 as the last page with []
    Then the JSON pointer "/cluster/copies_stored_total" at "/api/v1/stats" should eventually be "1"
    When the board "02:00:00:0a:0b:02" stops pulsing
    And I restart the host stack
    And I send a GET request to "/api/v1/cluster/channels/16"
    Then the response status should be 200
    And the JSON pointer "/name" should be "Hall"
    And the JSON pointer "/home_node" should be 1
    And the JSON pointer "/birth_mark/nonce" should be 5
    And the JSON pointer "/source" should be "copy"

  @id:CLU-141
  Scenario: A rename on the home node reaches the copy
    When I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/3"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/cluster/copies_stored_total" at "/api/v1/stats" should eventually be "1"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 2 as the last page with [{"channel":16,"mark":[1,5],"name":"Lobby","revision":2}]
    Then the JSON pointer "/cluster/copies_stored_total" at "/api/v1/stats" should eventually be "2"
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "false"
    When I send a DELETE request to "/api/v1/cluster/nodes/1"
    Then the response status should be 204
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the JSON pointer "/name" should be "Lobby"
    And the JSON pointer "/revision" should be 2
    And the JSON pointer "/source" should be "copy"

  @id:CLU-142
  Scenario: A lower revision on the home node does not lower the copy
    When I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/3"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Lobby","revision":3}]
    Then the JSON pointer "/cluster/copies_stored_total" at "/api/v1/stats" should eventually be "1"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 2 as the last page with [{"channel":16,"mark":[1,5],"name":"Old","revision":2}]
    Then the JSON pointer "/name" at "/api/v1/cluster/channels/16" should eventually be "Old"
    And the JSON pointer "/cluster/copies_stored_total" at "/api/v1/stats" should stay "1" for 1000 ms
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "false"
    When I send a DELETE request to "/api/v1/cluster/nodes/1"
    And I send a GET request to "/api/v1/cluster/channels/16"
    Then the JSON pointer "/name" should be "Lobby"
    And the JSON pointer "/revision" should be 3

  @id:CLU-143
  Scenario: A record known only by its copy is listed under the copy's name with its home node not heard
    When I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/3"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/cluster/copies_stored_total" at "/api/v1/stats" should eventually be "1"
    When I send a GET request to "/api/v1/cluster/channels"
    Then the JSON pointer "/channels/0/source" should be "cached"
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "false"
    When I send a DELETE request to "/api/v1/cluster/nodes/1"
    And I send a GET request to "/api/v1/cluster/channels"
    Then the JSON pointer "/channels/0/number" should be 16
    And the JSON pointer "/channels/0/name" should be "Hall"
    And the JSON pointer "/channels/0/home_node" should be 1
    And the JSON pointer "/channels/0/source" should be "copy"
    And the JSON pointer "/channels/1" should be absent
    When I send a GET request to "/api/v1/cluster/channels/16"
    Then the JSON pointer "/parts/0/node" should be 0
    And the JSON pointer "/parts/0/link" should be "unknown"
    When I send a GET request to "/api/v1/cluster/info"
    Then the JSON pointer "/nodes/1" should be absent

  @id:CLU-144
  Scenario: A part is judged an orphan only from its settled home's whole set, and goes only when it is deleted
    When I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/3"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"}]
    Then the JSON pointer "/channel_parts/0/link" at "/api/v1/cluster/nodes/0/channel-parts" should eventually be "live"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 2 as a page before the last with []
    Then the JSON pointer "/channel_parts/0/link" at "/api/v1/cluster/nodes/0/channel-parts" should stay "live" for 500 ms
    When the board "02:00:00:0a:0b:02" sends info page 1 of set 2 as the last page with []
    Then the JSON pointer "/channel_parts/0/link" at "/api/v1/cluster/nodes/0/channel-parts" should eventually be "unknown"
    When the board "02:00:00:0a:0b:02" announces one-page sets from 3 with [] until the JSON pointer "/channel_parts/0/link" at "/api/v1/cluster/nodes/0/channel-parts" is "orphan"
    And I send a GET request to "/api/v1/cluster/channel-parts/0/3"
    Then the response status should be 200
    And the JSON pointer "/record/number" should be 16
    When I send a DELETE request to "/api/v1/cluster/channel-parts/0/3"
    Then the response status should be 204
    When I send a GET request to "/api/v1/cluster/nodes/0/channel-parts"
    Then the JSON pointer "/channel_parts/0" should be absent

  @id:CLU-145
  Scenario: A part whose neighbour on the same wire holds another record or none is a wire fault and changes nothing
    When I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/3"
    And I PUT JSON {"record":{"number":17,"birth_mark":{"generation":1,"nonce":6}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/4"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 on the wire of node 0 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":16,"mark":[1,5],"name":"Hall"},{"channel":17,"mark":[1,6],"name":"Porch"},{"channel_part":[0,3],"record":17,"mark":[1,6],"send":true,"accept":true}]
    Then the JSON pointer "/channel_parts/0/wire_fault/holds" at "/api/v1/cluster/nodes/0/channel-parts" should eventually be "other_record"
    When I send a GET request to "/api/v1/cluster/nodes/0/channel-parts"
    Then the JSON pointer "/channel_parts/0/group" should be 3
    And the JSON pointer "/channel_parts/0/wire_fault/node" should be 1
    And the JSON pointer "/channel_parts/1/group" should be 4
    And the JSON pointer "/channel_parts/1/wire_fault/node" should be 1
    And the JSON pointer "/channel_parts/1/wire_fault/holds" should be "nothing"
    When I send a GET request to "/api/v1/cluster/nodes/1/channel-parts"
    Then the JSON pointer "/channel_parts/0/wire_fault/node" should be 0
    And the JSON pointer "/channel_parts/0/wire_fault/holds" should be "other_record"
    When I send a GET request to "/api/v1/cluster/channels/17"
    Then the JSON pointer "/parts/0/node" should be 0
    And the JSON pointer "/parts/0/wire_fault/holds" should be "nothing"
    And the JSON pointer "/parts/1/node" should be 1
    And the JSON pointer "/parts/1/wire_fault/holds" should be "other_record"
    When I send a GET request to "/api/v1/cluster/channel-parts/0/3"
    Then the JSON pointer "/record/number" should be 16

  @id:CLU-146
  Scenario: A neighbour's forged copies and records outside its block change nothing they must not
    When I PUT JSON {"record":{"number":32,"birth_mark":{"generation":1,"nonce":7}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/3"
    And a board "02:00:00:0a:0b:03" at "10.0.0.3" pulses as node 2 of cluster "hall" for installation "site-c"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/1/heard" at "/api/v1/cluster/status" should eventually be "true"
    When the board "02:00:00:0a:0b:03" sends info page 0 of set 1 as the last page with [{"channel":32,"mark":[1,7],"name":"Porch"}]
    Then the JSON pointer "/cluster/copies_stored_total" at "/api/v1/stats" should eventually be "1"
    When the board "02:00:00:0a:0b:02" sends info page 0 of set 1 as the last page with [{"channel":32,"mark":[1,7],"name":"Forged","revision":9},{"channel":32,"mark":[1,7],"name":"Same","revision":1},{"channel":32,"mark":[2,8],"name":"Other mark","revision":9},{"channel":3,"mark":[5,1],"name":"Into this block","revision":9}]
    Then the JSON pointer "/nodes/1/skipped" at "/api/v1/cluster/info" should eventually be "3"
    And the JSON pointer "/cluster/copies_stored_total" at "/api/v1/stats" should stay "1" for 1000 ms
    And the JSON pointer "/cluster/records_taken_back_total" at "/api/v1/stats" should eventually be "0"
    When I send a GET request to "/api/v1/cluster/channels/3"
    Then the response status should be 404
    When I send a GET request to "/api/v1/cluster/channels/32"
    Then the JSON pointer "/name" should be "Porch"
    When the board "02:00:00:0a:0b:03" stops pulsing
    Then the JSON pointer "/nodes/1/heard" at "/api/v1/cluster/status" should eventually be "false"
    When I send a DELETE request to "/api/v1/cluster/nodes/2"
    And I send a GET request to "/api/v1/cluster/channels/32"
    Then the JSON pointer "/name" should be "Porch"
    And the JSON pointer "/source" should be "copy"
