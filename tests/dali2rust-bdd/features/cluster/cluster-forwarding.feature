@stage-I6
Feature: Cluster — a command given to this node's group, all its lamps or a scene goes to the cluster

  A command executed here on a group whose channel part sends, on all lamps
  while this node sends to all lamps, or a scene recall whose scene part sends,
  goes to the cluster as the command to the channel, to all lamps or to the
  cluster scene, stamped with this node's wire and not executed here again. The
  operator and Home Assistant are forwarded out of the box, HCL and the foreign
  master when they are switched on; rules and the cluster itself never are. A command a
  neighbour sent and this node executed goes nowhere. A neighbour's command to
  all lamps runs here as a broadcast only while this node accepts all lamps, and
  a neighbour's HCL tick keeps the wire priority of an HCL tick here and does
  not stand down this node's schedule. The host stack holds two logical
  adapters on one mock wire, so a broadcast on every adapter is two frames.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"

  @id:CLU-090
  Scenario: An operator's group command goes to the cluster as the channel's command and runs here once
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":200} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the cluster hub should carry from this controller the command {"channel":[16,1,5],"level":200,"executed_on_wire":0} twice with one sequence number
    And the DALI mock transport forward trace should eventually be exactly "8EC8"
    And the JSON pointer "/cluster/commands_forwarded_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_sent_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-091
  Scenario Outline: A group command goes nowhere while its group has no sending part
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":<send>,"accept":true} to "/api/v1/cluster/channel-parts/0/<group>"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":200} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the JSON pointer "/cluster/forwards_skipped_not_sent_total" at "/api/v1/stats" should eventually be "1"
    And the DALI mock transport forward trace should eventually be exactly "8EC8"
    And the cluster hub should carry no command from this controller within 1000 ms

    Examples:
      | send  | group |
      | false | 7     |
      | true  | 8     |

  @id:CLU-092
  Scenario: A command to one lamp goes nowhere, and the group command after it is the first on the network
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"send_all_lamps":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":220} to "/api/v1/adapters/0/virtual-lamps/1/target-state"
    Then the response status should be 200
    And the DALI mock transport forward trace should eventually be exactly "00DC"
    When I PUT JSON {"power":"on","level":200} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the cluster hub should carry from this controller the command {"channel":[16,1,5],"level":200,"executed_on_wire":0} twice with one sequence number
    And the JSON pointer "/cluster/commands_forwarded_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-093
  Scenario: An operator's group command goes nowhere while the operator is not forwarded
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_operator":false} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":200} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the JSON pointer "/cluster/forwards_skipped_source_off_total" at "/api/v1/stats" should eventually be "1"
    And the DALI mock transport forward trace should eventually be exactly "8EC8"
    And the cluster hub should carry no command from this controller within 1000 ms

  @id:CLU-094
  Scenario Outline: An HCL tick on a group goes to the cluster as an HCL-class command only while HCL is forwarded
    Given the controller clock reads 600 minutes past midnight
    And adapter 0 has discovered physical devices 0 and 1
    And physical device 0 reports membership of group 7 read from the gear
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_hcl":<forward_hcl>} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And I POST JSON {"schedule_id":"house","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[7]}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"absolute","level":80,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the scheduler should drive group 7 to level 80
    And the JSON pointer "/cluster/<counter>" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry <what>

    Examples:
      | forward_hcl | counter                           | what                                                                                                                        |
      | true        | commands_forwarded_total          | from this controller the command {"channel":[16,1,5],"level":80,"source":"hcl","executed_on_wire":0} twice with one sequence number |
      | false       | forwards_skipped_source_off_total | no command from this controller within 1000 ms                                                                              |

  @id:CLU-095
  Scenario: A foreign master's scene recall on a group goes nowhere from here while the foreign master is switched off
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":17,"birth_mark":{"generation":1,"nonce":6}},"send":true,"accept":true} to "/api/v1/cluster/scene-parts/0/4"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And a foreign group 7 recall of scene 4 is observed on the bus
    Then the JSON pointer "/cluster/forwards_skipped_source_off_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms
    When I POST JSON {"scope":"group","group_id":7} to "/api/v1/adapters/0/scenes/4/recall"
    Then the response status should be 200
    And the cluster hub should carry from this controller the command {"scene":[17,1,6],"channel":[16,1,5],"executed_on_wire":0} twice with one sequence number

  @id:CLU-096
  Scenario: A rule's group command goes nowhere
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When I PUT JSON {"base_revision":0,"source":"rule \"hall\" {\n  when http trigger\n  do group(7).level(120)\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I POST JSON {} to "/api/v1/rules/hall/run"
    Then the response status should be 202
    And the DALI mock transport forward trace should eventually be exactly "8E78"
    And the JSON pointer "/cluster/forwards_skipped_source_off_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms

  @id:CLU-097
  Scenario Outline: An HCL tick on all lamps goes to the cluster as a command to all lamps only while this node sends to all lamps
    Given the controller clock reads 600 minutes past midnight
    And adapter 0 has discovered physical devices 0 and 1
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_hcl":true,"send_all_lamps":<send_all>} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When the cluster hub sets aside what this controller has sent so far
    And I POST JSON {"schedule_id":"house","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"broadcast"}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"absolute","level":80,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the scheduler should drive broadcast to level 80
    And the JSON pointer "/cluster/<counter>" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry <what>

    Examples:
      | send_all | counter                         | what                                                                                                             |
      | true     | commands_forwarded_total        | from this controller the command {"all":true,"level":80,"source":"hcl","executed_on_wire":0} twice with one sequence number |
      | false    | forwards_skipped_not_sent_total | no command from this controller within 1000 ms                                                                   |

  @id:CLU-098
  Scenario Outline: A neighbour's command to all lamps runs here as a broadcast on every adapter only while this node accepts all lamps and owns its wire
    When I PATCH JSON <settings> to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" sends the command {"all":true,"level":200} twice with one sequence number
    Then the JSON pointer "/cluster/<counter>" at "/api/v1/stats" should eventually be "<times>"
    And the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the DALI mock transport <wire>
    And the cluster hub should carry no command from this controller within 500 ms

    Examples:
      | settings                                                                                         | counter                               | times | wire                                                    |
      | {"cluster_name":"hall","node_number":0,"enabled":true,"accept_all_lamps":true}                   | commands_executed_total               | 2     | forward trace should eventually be exactly "FEC8, FEC8" |
      | {"cluster_name":"hall","node_number":0,"enabled":true}                                           | commands_dropped_accept_off_total     | 1     | should have received 0 forward frames                   |
      | {"cluster_name":"hall","node_number":2,"wire_same_as":1,"enabled":true,"accept_all_lamps":true}  | commands_dropped_not_wire_owner_total | 1     | should have received 0 forward frames                   |

  @id:CLU-099
  Scenario Outline: A scene recalled here goes to the cluster on the whole node or on the group's channel
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":17,"birth_mark":{"generation":1,"nonce":6}},"send":true,"accept":true} to "/api/v1/cluster/scene-parts/0/4"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":<group_sends>,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I POST JSON <recall> to "/api/v1/adapters/0/scenes/4/recall"
    Then the response status should be 200
    And the DALI mock transport forward trace should eventually be exactly "<trace>"
    And the JSON pointer "/cluster/<counter>" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry <what>

    Examples:
      | group_sends | recall                          | trace | counter                         | what                                                                                                       |
      | false       | {}                              | FF14  | commands_forwarded_total        | from this controller the command {"scene":[17,1,6],"executed_on_wire":0} twice with one sequence number                   |
      | true        | {"scope":"group","group_id":7}  | 8F14  | commands_forwarded_total        | from this controller the command {"scene":[17,1,6],"channel":[16,1,5],"executed_on_wire":0} twice with one sequence number |
      | false       | {"scope":"group","group_id":7}  | 8F14  | forwards_skipped_not_sent_total | no command from this controller within 1000 ms                                                             |

  @id:CLU-100
  Scenario Outline: A neighbour's HCL tick keeps an HCL tick's priority here and does not stand down this node's schedule
    Given the controller clock reads 600 minutes past midnight
    And adapter 0 has discovered physical devices 0 and 1
    And physical device 0 reports membership of group 7 read from the gear
    When I PUT JSON {"physical_short_address":0} to "/api/v1/adapters/0/virtual-lamps/1/binding"
    Then the response status should be 200
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    When I POST JSON {"schedule_id":"house","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri","sat","sun"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[7]}],"points":[{"time_ref":"absolute","offset_minutes":540,"level_mode":"absolute","level":80,"color_temperature_kelvin":null}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the scheduler should drive group 7 to level 80
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":120,"source":"<source>"} twice with one sequence number
    Then the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the DALI mock transport should have sent group 7 direct arc level 120
    And the first forward frame should be sent at priority <priority>
    And virtual lamp 1 on adapter 0 should eventually report level 120 from value_source "<value_source>"
    When the scheduler has run 3 more ticks
    Then HCL schedule "house" <verdict> group 7 suspended
    And the cluster hub should carry no command from this controller within 500 ms

    Examples:
      | source   | priority | value_source | verdict             |
      | hcl      | 4        | cluster_hcl  | should not report   |
      | operator | 2        | cluster      | eventually reports  |
