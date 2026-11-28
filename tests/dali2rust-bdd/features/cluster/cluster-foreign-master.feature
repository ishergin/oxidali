@stage-I6
Feature: Cluster — the cluster follows a foreign master

  A foreign master's group command, broadcast or scene recall heard on this
  node's wire goes to the cluster as the channel's command, the command to all
  lamps or the cluster scene's recall, with the source class foreign master and
  this node's wire stamped as executed, while the foreign master is switched on.
  Only the wire's owner forwards it, a command to one lamp never goes, and the
  foreign frame is not executed here again. With a neighbour on the wire the
  owner holds each observation for a while: the neighbour's cluster command
  executed on this wire with the same target and verb cancels it, one for one
  for a relative verb. Alone on its wire a node holds nothing. A neighbour on
  this wire without loop protection stops the foreign master here and is
  reported in the status, and protection cannot be switched off while a
  neighbour names this node's wire.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"

  @id:CLU-170
  Scenario Outline: Each foreign verb on a group goes to the cluster as the channel's command and runs here not again
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0x<address> 0x<command> is observed on the bus
    Then the cluster hub should carry from this controller the command <forwarded> twice with one sequence number
    And the JSON pointer "/cluster/foreign_forwarded_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/foreign_held_total" at "/api/v1/stats" should eventually be "0"
    And no DALI frame should reach the bus within 300 ms

    Examples:
      | address | command | forwarded                                                                                         |
      | 8E      | C8      | {"channel":[16,1,5],"level":200,"source":"foreign_master","executed_on_wire":0}                   |
      | 8F      | 00      | {"channel":[16,1,5],"power":"off","level":0,"source":"foreign_master","executed_on_wire":0}       |
      | 8F      | 01      | {"channel":[16,1,5],"dim":"up","source":"foreign_master","executed_on_wire":0}                    |
      | 8F      | 02      | {"channel":[16,1,5],"dim":"down","source":"foreign_master","executed_on_wire":0}                  |
      | 8F      | 03      | {"channel":[16,1,5],"dim":"step_up","source":"foreign_master","executed_on_wire":0}               |
      | 8F      | 04      | {"channel":[16,1,5],"dim":"step_down","source":"foreign_master","executed_on_wire":0}             |
      | 8F      | 05      | {"channel":[16,1,5],"recall":"max_level","source":"foreign_master","executed_on_wire":0}          |
      | 8F      | 06      | {"channel":[16,1,5],"recall":"min_level","source":"foreign_master","executed_on_wire":0}          |
      | 8F      | 07      | {"channel":[16,1,5],"dim":"step_down_and_off","source":"foreign_master","executed_on_wire":0}     |
      | 8F      | 08      | {"channel":[16,1,5],"dim":"on_and_step_up","source":"foreign_master","executed_on_wire":0}        |
      | 8F      | 0A      | {"channel":[16,1,5],"power":"on","source":"foreign_master","executed_on_wire":0}                  |
      | 8E      | FF      | {"channel":[16,1,5],"dim":"stop_fade","source":"foreign_master","executed_on_wire":0}             |

  @id:CLU-171
  Scenario Outline: A foreign broadcast goes to all lamps only while this node sends to all lamps
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true,"send_all_lamps":<send_all>} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0xFE 0xC8 is observed on the bus
    Then the JSON pointer "/cluster/<counter>" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry <carried>

    Examples:
      | send_all | counter                         | carried                                                                                                                   |
      | true     | foreign_forwarded_total         | from this controller the command {"all":true,"level":200,"source":"foreign_master","executed_on_wire":0} twice with one sequence number |
      | false    | forwards_skipped_not_sent_total | no command from this controller within 1000 ms                                                                            |

  @id:CLU-172
  Scenario: A foreign master's scene recall on a group goes to the cluster as the cluster scene's recall
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":17,"birth_mark":{"generation":1,"nonce":6}},"send":true,"accept":true} to "/api/v1/cluster/scene-parts/0/4"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign group 7 recall of scene 4 is observed on the bus
    Then the cluster hub should carry from this controller the command {"scene":[17,1,6],"channel":[16,1,5],"source":"foreign_master","executed_on_wire":0} twice with one sequence number
    And no DALI frame should reach the bus within 300 ms

  @id:CLU-173
  Scenario: A foreign command to one lamp goes nowhere, and the group command after it is the first on the network
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0x06 0xC8 is observed on the bus
    And a foreign raw forward frame 0x8F 0x01 is observed on the bus
    Then the cluster hub should carry from this controller the command {"channel":[16,1,5],"dim":"up","source":"foreign_master","executed_on_wire":0} twice with one sequence number
    And the cluster hub should carry no command from this controller within 1000 ms
    And the JSON pointer "/cluster/foreign_forwarded_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-174
  Scenario Outline: A foreign group command goes nowhere while the foreign master is off or the group's part does not send
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":<foreign>} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":<send>,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0x8F 0x01 is observed on the bus
    Then the JSON pointer "/cluster/<counter>" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms

    Examples:
      | foreign | send  | counter                           |
      | false   | true  | forwards_skipped_source_off_total |
      | true    | false | forwards_skipped_not_sent_total   |

  @id:CLU-175
  Scenario: A node that does not own its wire forwards no foreign command
    When I PATCH JSON {"cluster_name":"hall","node_number":1,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":2,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 0 on the wire of node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/wire_owner/this_node" at "/api/v1/cluster/status" should eventually be "false"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0x8F 0x01 is observed on the bus
    Then the JSON pointer "/cluster/forwards_skipped_wire_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms

  @id:CLU-176
  Scenario: Alone on its wire the owner holds nothing and the foreign command leaves at once
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/wire_owner/this_node" at "/api/v1/cluster/status" should eventually be "true"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0x8F 0x01 is observed on the bus
    Then the cluster hub should carry from this controller the command {"channel":[16,1,5],"dim":"up","source":"foreign_master","executed_on_wire":0} twice with one sequence number
    And the JSON pointer "/cluster/foreign_held_total" at "/api/v1/stats" should eventually be "0"
    And the JSON pointer "/cluster/foreign_forwarded_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-177
  Scenario: With a neighbour on the wire the owner holds a foreign command and sends it after the hold
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 on the wire of node 0 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/wire_owner/this_node" at "/api/v1/cluster/status" should eventually be "true"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0x8F 0x01 is observed on the bus
    Then the cluster hub should carry no command from this controller within 150 ms
    And the cluster hub should carry from this controller the command {"channel":[16,1,5],"dim":"up","source":"foreign_master","executed_on_wire":0} twice with one sequence number
    And the JSON pointer "/cluster/foreign_held_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/foreign_forwarded_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-178
  Scenario Outline: The neighbour's command executed on this wire with the same target and verb cancels the held one, and another does not
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 on the wire of node 0 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/wire_owner/this_node" at "/api/v1/cluster/status" should eventually be "true"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0x<address> 0x<command> is observed on the bus
    And the board "02:00:00:0a:0b:02" sends the command <neighbour> twice with one sequence number
    Then the JSON pointer "/cluster/commands_dropped_executed_on_wire_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/foreign_cancelled_total" at "/api/v1/stats" should eventually be "<cancelled>"
    And the cluster hub should carry <carried>

    Examples:
      | address | command | neighbour                                                                     | cancelled | carried                                                                                                                                       |
      | 8F      | 01      | {"channel":[16,1,5],"dim":"up","source":"foreign_master","executed_on_wire":0}   | 1         | no command from this controller within 1000 ms                                                                                                |
      | 8F      | 01      | {"channel":[16,1,5],"dim":"down","source":"foreign_master","executed_on_wire":0} | 0         | from this controller the command {"channel":[16,1,5],"dim":"up","source":"foreign_master","executed_on_wire":0} twice with one sequence number |
      | 8E      | C8      | {"channel":[16,1,5],"level":200,"executed_on_wire":0}                            | 1         | no command from this controller within 1000 ms                                                                                                |
      | 8E      | C8      | {"channel":[16,1,5],"level":100,"executed_on_wire":0}                            | 0         | from this controller the command {"channel":[16,1,5],"level":200,"source":"foreign_master","executed_on_wire":0} twice with one sequence number |

  @id:CLU-179
  Scenario: Two foreign UPs and one neighbour UP leave one UP
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 on the wire of node 0 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/wire_owner/this_node" at "/api/v1/cluster/status" should eventually be "true"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0x8F 0x01 is observed on the bus
    And a foreign raw forward frame 0x8F 0x01 is observed on the bus
    And the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"dim":"up","source":"foreign_master","executed_on_wire":0} twice with one sequence number
    Then the cluster hub should carry from this controller the command {"channel":[16,1,5],"dim":"up","source":"foreign_master","executed_on_wire":0} twice with one sequence number
    And the cluster hub should carry no command from this controller within 1000 ms
    And the JSON pointer "/cluster/foreign_cancelled_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/foreign_forwarded_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-185
  Scenario: A neighbour on this wire without loop protection stops the foreign master here until it is gone
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 on the wire of node 0 without wire protection of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/wire_protection" at "/api/v1/cluster/status" should eventually be "false"
    And the JSON pointer "/own/unprotected_wire" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign raw forward frame 0x8F 0x01 is observed on the bus
    Then the JSON pointer "/cluster/forwards_skipped_wire_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms
    When the board "02:00:00:0a:0b:02" stops pulsing
    Then the JSON pointer "/own/unprotected_wire" at "/api/v1/cluster/status" should eventually be "false"
    When a foreign raw forward frame 0x8F 0x01 is observed on the bus
    Then the cluster hub should carry from this controller the command {"channel":[16,1,5],"dim":"up","source":"foreign_master","executed_on_wire":0} twice with one sequence number

  @id:CLU-186
  Scenario: Loop protection cannot be switched off while the foreign master is forwarded and a neighbour names this node's wire
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 on the wire of node 0 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    When I PATCH JSON {"wire_protection":false} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON field "message" should be "wire_protection_required"
    When I send a GET request to "/api/v1/cluster"
    Then the JSON boolean field "wire_protection" should be true
    When I PATCH JSON {"forward_foreign_master":false,"wire_protection":false} to "/api/v1/cluster"
    Then the response status should be 200

  @id:CLU-187
  Scenario Outline: A cluster command executed here goes nowhere, with loop protection on and off
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_foreign_master":true,"wire_protection":<protection>} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":200,"executed_on_wire":1} twice with one sequence number
    Then the DALI mock transport forward trace should eventually be exactly "8EC8"
    And the JSON pointer "/cluster/forwards_skipped_cluster_origin_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms
    And the JSON pointer "/cluster/foreign_forwarded_total" at "/api/v1/stats" should eventually be "0"

    Examples:
      | protection |
      | true       |
      | false      |
