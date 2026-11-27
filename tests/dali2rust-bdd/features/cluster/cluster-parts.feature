@stage-I6
Feature: Cluster — this node's parts of channels and cluster scenes

  A part is this node's share in a channel or a cluster scene: one DALI group,
  or one scene, of one of its adapters, linked as it is to a record, with the
  flags to send and to accept. A part names its record by number and birth
  mark, so a record of another node is named as given, and a record of this
  node only as it lives. One group or scene has at most one part, and one
  record at most one part on each adapter. Parts are a setting: a write is
  applied before it is answered.

  @id:CLU-045
  Scenario Outline: A part links a group or scene of this node to another node's record as given
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":4,"nonce":77}},"send":true,"accept":false} to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    And the JSON pointer "/adapter" should be 0
    And the JSON pointer "/<local>" should be 3
    And the JSON pointer "/record/number" should be 20
    And the JSON pointer "/record/birth_mark/generation" should be 4
    And the JSON pointer "/record/birth_mark/nonce" should be 77
    And the JSON pointer "/send" should be true
    And the JSON pointer "/accept" should be false
    And the JSON pointer "/revision" should be 1
    When I send a GET request to "/api/v1/cluster/<parts>"
    Then the response status should be 200
    And the JSON pointer "/<list>/0/<local>" should be 3
    And the JSON pointer "/<list>/0/record/number" should be 20
    And the JSON pointer "/<list>/1" should be absent
    When I send a GET request to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    And the JSON pointer "/record/birth_mark/nonce" should be 77

    Examples:
      | parts         | local | list          |
      | channel-parts | group | channel_parts |
      | scene-parts   | scene | scene_parts   |

  @id:CLU-046
  Scenario Outline: A part names a record of this node only as it lives
    When I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    When I link "/api/v1/cluster/<parts>/0/3" to the cluster record "/api/v1/cluster/<kind>/2" with send true and accept true
    Then the response status should be 200
    And the JSON pointer "/record/number" should be 2
    And the JSON pointer "/record/birth_mark/generation" should be 1
    When I PUT JSON {"record":{"number":2,"birth_mark":{"generation":9,"nonce":1}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/4"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "cluster_record_not_live"
    When I PUT JSON {"record":{"number":5,"birth_mark":{"generation":1,"nonce":1}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/4"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_record_not_live"
    When I send a GET request to "/api/v1/cluster/<parts>/0/4"
    Then the response status should be 404

    Examples:
      | kind     | parts         |
      | channels | channel-parts |
      | scenes   | scene-parts   |

  @id:CLU-047
  Scenario Outline: A changed part raises its revision and the same part written again keeps it
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":false} to "/api/v1/cluster/<parts>/0/3"
    And I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":false} to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    And the JSON pointer "/revision" should be 1
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    And the JSON pointer "/accept" should be true
    And the JSON pointer "/revision" should be 2
    When I PUT JSON {"record":{"number":21,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    And the JSON pointer "/record/number" should be 21
    And the JSON pointer "/revision" should be 3

    Examples:
      | parts         |
      | channel-parts |
      | scene-parts   |

  @id:CLU-048
  Scenario Outline: One record has at most one part on each adapter, and one group or scene one record
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/4"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "cluster_record_linked"
    When I PUT JSON {"record":{"number":21,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/4"
    Then the response status should be 200
    When I send a GET request to "/api/v1/cluster/<parts>"
    Then the JSON pointer "/<list>/0/record/number" should be 21
    And the JSON pointer "/<list>/1/record/number" should be 20
    And the JSON pointer "/<list>/2" should be absent

    Examples:
      | parts         | list          |
      | channel-parts | channel_parts |
      | scene-parts   | scene_parts   |

  @id:CLU-049
  Scenario: A group and a scene of the same number are parts of different kinds
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/channel-parts/0/3"
    Then the response status should be 200
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":false,"accept":true} to "/api/v1/cluster/scene-parts/0/3"
    Then the response status should be 200
    When I send a GET request to "/api/v1/cluster/channel-parts/0/3"
    Then the JSON pointer "/send" should be true
    When I send a GET request to "/api/v1/cluster/scene-parts/0/3"
    Then the JSON pointer "/send" should be false

  @id:CLU-050
  Scenario Outline: A deleted part is gone and a second delete finds nothing
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/3"
    And I send a DELETE request to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 204
    When I send a GET request to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 404
    When I send a DELETE request to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 404
    When I send a GET request to "/api/v1/cluster/<parts>"
    Then the JSON pointer "/<list>/0" should be absent

    Examples:
      | parts         | list          |
      | channel-parts | channel_parts |
      | scene-parts   | scene_parts   |

  @id:CLU-051
  Scenario Outline: A part of an adapter this node does not have is not found, and a group past the last is no group
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/7/3"
    Then the response status should be 404
    And the JSON error should be "not_found"
    When I send a GET request to "/api/v1/cluster/<parts>/7/3"
    Then the response status should be 404
    When I send a GET request to "/api/v1/cluster/<parts>/0/16"
    Then the response status should be 400
    And the JSON error should be "invalid_resource_id"
    When I send a GET request to "/api/v1/cluster/<parts>/8/3"
    Then the response status should be 400
    And the JSON error should be "invalid_resource_id"

    Examples:
      | parts         |
      | channel-parts |
      | scene-parts   |

  @id:CLU-052
  Scenario Outline: A part outlives the deletion of the record of this node it names
    When I POST JSON {"number":2} to "/api/v1/cluster/<kind>"
    And I link "/api/v1/cluster/<parts>/0/3" to the cluster record "/api/v1/cluster/<kind>/2" with send true and accept true
    And I DELETE JSON {"revision":1} to "/api/v1/cluster/<kind>/2"
    Then the response status should be 204
    When I POST JSON {"number":2} to "/api/v1/cluster/<kind>"
    Then the JSON pointer "/birth_mark/generation" should be 2
    When I send a GET request to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    And the JSON pointer "/record/number" should be 2
    And the JSON pointer "/record/birth_mark/generation" should be 1

    Examples:
      | kind     | parts         |
      | channels | channel-parts |
      | scenes   | scene-parts   |

  @id:CLU-053
  Scenario Outline: Parts survive a controller restart
    Given in-memory slice persistence is enabled for the host stack
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":3,"nonce":4000000000}},"send":false,"accept":true} to "/api/v1/cluster/<parts>/0/15"
    And I PUT JSON {"record":{"number":20,"birth_mark":{"generation":3,"nonce":4000000000}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/15"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/cluster/<parts>/0/15"
    Then the response status should be 200
    And the JSON pointer "/record/number" should be 20
    And the JSON pointer "/record/birth_mark/generation" should be 3
    And the JSON pointer "/record/birth_mark/nonce" should be 4000000000
    And the JSON pointer "/send" should be true
    And the JSON pointer "/revision" should be 2

    Examples:
      | parts         |
      | channel-parts |
      | scene-parts   |

  @id:CLU-054
  Scenario Outline: A standby reads the parts and refuses to write them
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    Given this controller has stood down
    When I send a GET request to "/api/v1/cluster/<parts>"
    Then the response status should be 200
    When I <method> JSON <body> to "/api/v1/cluster/<parts>/0/<local>"
    Then the response status should be 409
    And the JSON error should be "controller_standby"
    And the response header "X-Dali2rust-Role" should be "standby"
    When I send a GET request to "/api/v1/cluster/<parts>/0/3"
    Then the JSON pointer "/accept" should be true

    Examples:
      | parts         | method | local | body                                                                                     |
      | channel-parts | PUT    | 3     | {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":false} |
      | channel-parts | PUT    | 4     | {"record":{"number":21,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":false} |
      | channel-parts | DELETE | 3     | {}                                                                                       |
      | scene-parts   | DELETE | 3     | {}                                                                                       |
      | scene-parts   | DELETE | 4     | {}                                                                                       |

  @id:CLU-055
  Scenario Outline: A part write refuses a body it does not take
    When I PUT JSON {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true} to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be 200
    When I PUT JSON <body> to "/api/v1/cluster/<parts>/0/3"
    Then the response status should be <status>
    And the JSON error should be "<error>"
    When I send a GET request to "/api/v1/cluster/<parts>/0/3"
    Then the JSON pointer "/record/number" should be 20
    And the JSON pointer "/revision" should be 1

    Examples:
      | parts         | body                                                                                                 | status | error             |
      | channel-parts | {"record":{"number":128,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true}             | 422    | invalid_value     |
      | channel-parts | {"record":{"number":20,"birth_mark":{"generation":0,"nonce":5}},"send":true,"accept":true}              | 422    | invalid_value     |
      | channel-parts | {"record":{"number":20,"birth_mark":{"generation":1}},"send":true,"accept":true}                        | 422    | invalid_value     |
      | channel-parts | {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"accept":true}                          | 422    | invalid_value     |
      | channel-parts | {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":"yes","accept":true}             | 422    | invalid_value     |
      | channel-parts | {"record":null,"send":true,"accept":true}                                                               | 422    | invalid_value     |
      | channel-parts | {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true,"colour":1}   | 400    | unknown_field     |
      | channel-parts | {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true,"revision":1} | 422    | unsupported_field |
      | scene-parts   | {"record":{"number":20,"birth_mark":{"generation":1,"nonce":5}},"send":true,"accept":true,"adapter":0}  | 422    | unsupported_field |
      | scene-parts   | {"record":{"number":20,"birth_mark":{"generation":1,"nonce":4294967296}},"send":true,"accept":true}     | 422    | invalid_value     |
