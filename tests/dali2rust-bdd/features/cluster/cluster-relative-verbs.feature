@stage-I6
Feature: Cluster — UP, DOWN, the steps and a stopped fade between nodes

  A relative verb given to this node's group goes to the cluster as the verb
  to the group's channel, by the same rules as a setpoint, and each node that
  accepts the channel runs it on its own group as its own DALI command. Two
  steps are two steps on every wire: the sender never folds a relative verb
  into another, sends them in their order and lets a setpoint given after
  them land last. A burst faster than the sender's spacing waits in a short
  line, and a relative verb beyond the line is dropped and counted, never
  sent late. A lost relative verb is not repaired: the segments stay apart
  until the next absolute command brings them together.

  Background:
    Given the controller runs on a cluster hub as board "02:00:00:0a:0b:01" at "10.0.0.1"

  @id:CLU-101
  Scenario Outline: An operator's relative verb on a group goes to the cluster as the channel's verb and runs here once
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"dim":"<dim>"} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the cluster hub should carry from this controller the command {"channel":[16,1,5],"dim":"<dim>","hold_hcl":<hold_hcl>,"executed_on_wire":0} twice with one sequence number
    And the DALI mock transport forward trace should eventually be exactly "<trace>"
    And the JSON pointer "/cluster/commands_sent_total" at "/api/v1/stats" should eventually be "1"

    Examples:
      | dim               | hold_hcl | trace |
      | up                | true     | 8F01  |
      | down              | true     | 8F02  |
      | step_up           | true     | 8F03  |
      | step_down         | true     | 8F04  |
      | on_and_step_up    | true     | 8F08  |
      | step_down_and_off | true     | 8F07  |
      | stop_fade         | false    | 8EFF  |

  @id:CLU-102
  Scenario Outline: A neighbour's relative verb runs once on the linked group as its own command, its copy runs nothing and nothing goes back
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"dim":"<dim>"} twice with one sequence number
    Then the JSON pointer "/cluster/commands_received_total" at "/api/v1/stats" should eventually be "2"
    And the JSON pointer "/cluster/commands_dropped_duplicate_total" at "/api/v1/stats" should eventually be "1"
    And the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "1"
    And the DALI mock transport forward trace should eventually be exactly "<trace>"
    And the JSON pointer "/cluster/forwards_skipped_cluster_origin_total" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms

    Examples:
      | dim               | trace |
      | up                | 8F01  |
      | down              | 8F02  |
      | step_up           | 8F03  |
      | step_down         | 8F04  |
      | on_and_step_up    | 8F08  |
      | step_down_and_off | 8F07  |
      | stop_fade         | 8EFF  |

  @id:CLU-103
  Scenario: Two quick steps from a neighbour run here as two steps
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"dim":"step_up"} twice with one sequence number
    And the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"dim":"step_up"} twice with one sequence number
    Then the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "2"
    And the JSON pointer "/cluster/commands_dropped_duplicate_total" at "/api/v1/stats" should eventually be "2"
    And the DALI mock transport forward trace should eventually be exactly "8F03, 8F03"

  @id:CLU-104
  Scenario: Two quick steps on a group here go to the cluster as two, in their order
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"dim":"step_up"} to "/api/v1/adapters/0/groups/7/target-state"
    And I PUT JSON {"dim":"step_down"} to "/api/v1/adapters/0/groups/7/target-state"
    Then the cluster hub should carry from this controller the commands [{"channel":[16,1,5],"dim":"step_up","executed_on_wire":0},{"channel":[16,1,5],"dim":"step_down","executed_on_wire":0}] in this order, each twice with one sequence number
    And the DALI mock transport forward trace should eventually be exactly "8F03, 8F04"
    And the JSON pointer "/cluster/commands_superseded_total" at "/api/v1/stats" should eventually be "0"

  @id:CLU-105
  Scenario: A setpoint given after steps lands last, and the setpoint before them still goes out
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"power":"on","level":100} to "/api/v1/adapters/0/groups/7/target-state"
    And I PUT JSON {"dim":"step_up"} to "/api/v1/adapters/0/groups/7/target-state"
    And I PUT JSON {"dim":"step_up"} to "/api/v1/adapters/0/groups/7/target-state"
    And I PUT JSON {"power":"on","level":200} to "/api/v1/adapters/0/groups/7/target-state"
    Then the cluster hub should carry from this controller the commands [{"channel":[16,1,5],"level":100,"executed_on_wire":0},{"channel":[16,1,5],"dim":"step_up","executed_on_wire":0},{"channel":[16,1,5],"dim":"step_up","executed_on_wire":0},{"channel":[16,1,5],"level":200,"executed_on_wire":0}] in this order, each twice with one sequence number
    And the DALI mock transport forward trace should eventually be exactly "8E64, 8F03, 8F03, 8EC8"

  @id:CLU-106
  Scenario: UP held on a group goes to the cluster in its order and never more than the line holds
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"dim":"up"} to "/api/v1/adapters/0/groups/7/target-state" 8 times
    Then the DALI mock transport forward trace should eventually be exactly "8F01, 8F01, 8F01, 8F01, 8F01, 8F01, 8F01, 8F01"
    And the cluster hub should carry from this controller 8 forwarded commands {"channel":[16,1,5],"dim":"up","executed_on_wire":0} as sent or dropped, in their order, each twice with one sequence number

  @id:CLU-107
  Scenario: A channel takes a relative verb by the same body as a group, goes out and runs on this node's own part
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    Then the response status should be 201
    When I link "/api/v1/cluster/channel-parts/0/7" to the cluster record "/api/v1/cluster/channels/2" with send true and accept true
    Then the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"dim":"step_down"} to "/api/v1/cluster/channels/2/target-state"
    Then the response status should be 200
    And the JSON field "status" should be "sent"
    And the DALI mock transport forward trace should eventually be exactly "8F04"
    And the JSON pointer "/cluster/commands_sent_total" at "/api/v1/stats" should eventually be "1"

  @id:CLU-108
  Scenario Outline: A channel refuses a relative verb it cannot read, or one mixed with a setpoint
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/channels"
    Then the response status should be 201
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON <body> to "/api/v1/cluster/channels/2/target-state"
    Then the response status should be 422
    And the JSON error should be "<error>"
    And the cluster hub should carry no command from this controller within 500 ms

    Examples:
      | body                              | error         |
      | {"dim":"sideways"}                | invalid_enum  |
      | {"dim":"up","level":10}           | invalid_value |
      | {"dim":"up","recall":"max_level"} | invalid_value |

  @id:CLU-109
  Scenario: A neighbour's relative verb to all lamps runs here as a broadcast on every adapter
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"accept_all_lamps":true} to "/api/v1/cluster"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And the board "02:00:00:0a:0b:02" sends the command {"all":true,"dim":"up"} twice with one sequence number
    Then the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "2"
    And the DALI mock transport forward trace should eventually be exactly "FF01, FF01"
    And the cluster hub should carry no command from this controller within 500 ms

  @id:CLU-110
  Scenario Outline: A relative verb on a group goes nowhere while its part does not send or the operator is not forwarded
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true,"forward_operator":<forward_operator>} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":<send>,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the cluster hub sets aside what this controller has sent so far
    And I PUT JSON {"dim":"step_up"} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the DALI mock transport forward trace should eventually be exactly "8F03"
    And the JSON pointer "/cluster/<counter>" at "/api/v1/stats" should eventually be "1"
    And the cluster hub should carry no command from this controller within 1000 ms

    Examples:
      | send  | forward_operator | counter                           |
      | false | true             | forwards_skipped_not_sent_total   |
      | true  | false            | forwards_skipped_source_off_total |

  @id:CLU-111
  Scenario: A lost step leaves this segment behind until the next setpoint brings it level
    When I PATCH JSON {"cluster_name":"hall","node_number":0,"enabled":true} to "/api/v1/cluster"
    And I PUT JSON {"record":{"number":16,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/7"
    And a board "02:00:00:0a:0b:02" at "10.0.0.2" pulses as node 1 of cluster "hall" for installation "site-b"
    Then the JSON pointer "/nodes/0/heard" at "/api/v1/cluster/status" should eventually be "true"
    And the JSON pointer "/own/state" at "/api/v1/cluster/status" should eventually be "running"
    Given a DALI mock transport with no response
    When the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"dim":"step_up"} twice with one sequence number
    And the board "02:00:00:0a:0b:02" loses a command on the way
    And the board "02:00:00:0a:0b:02" sends the command {"channel":[16,1,5],"level":200} twice with one sequence number
    Then the JSON pointer "/cluster/commands_executed_total" at "/api/v1/stats" should eventually be "2"
    And the DALI mock transport forward trace should eventually be exactly "8F03, 8EC8"
