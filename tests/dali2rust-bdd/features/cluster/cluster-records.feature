@stage-I6
Feature: Cluster — channel and cluster-scene records on their home node

  A channel and a cluster scene are records kept by their home node: the node
  whose block of sixteen numbers gave the record its number. The home node
  issues the number, the birth mark that tells the record from an earlier one
  under the same number, and the name with its revision. A record of another
  node's block is not this node's to create.

  @id:CLU-015
  Scenario Outline: A record created without a number takes the lowest number of this node's block
    When I PATCH JSON {"node_number":2} to "/api/v1/cluster"
    Then the response status should be 200
    When I POST JSON {"name":"Corridor"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON numeric field "number" should be 32
    And the JSON numeric field "home_node" should be 2
    And the JSON field "name" should be "Corridor"
    And the JSON numeric field "revision" should be 1
    And the JSON pointer "/birth_mark/generation" should be 1
    When I POST JSON {"name":"Stairs"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON numeric field "number" should be 33
    When I send a GET request to "/api/v1/cluster/<kind>"
    Then the response status should be 200
    And the JSON pointer "/<kind>/0/number" should be 32
    And the JSON pointer "/<kind>/0/name" should be "Corridor"
    And the JSON pointer "/<kind>/1/number" should be 33
    And the JSON pointer "/<kind>/2" should be absent
    When I send a GET request to "/api/v1/cluster/<kind>/33"
    Then the response status should be 200
    And the JSON field "name" should be "Stairs"
    And the JSON numeric field "home_node" should be 2

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-016
  Scenario Outline: A record created without a name is named after its kind and number
    When I POST JSON {"number":4} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON field "name" should be "<name>"

    Examples:
      | kind     | name    |
      | channels | Channel 4 |
      | scenes   | Scene 4   |

  @id:CLU-017
  Scenario Outline: A number set by hand inside this node's block is issued as asked
    When I POST JSON {"number":9,"name":"Hall"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON numeric field "number" should be 9
    When I POST JSON {"name":"Next"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON numeric field "number" should be 0

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-018
  Scenario Outline: A number of another node's block is not this node's to issue
    When I POST JSON {"number":16,"name":"Hall"} to "/api/v1/cluster/<kind>"
    Then the response status should be 409
    And the JSON error should be "conflict"
    And the JSON field "message" should be "cluster_number_of_another_node"
    When I send a GET request to "/api/v1/cluster/<kind>"
    Then the JSON pointer "/<kind>/0" should be absent

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-019
  Scenario Outline: A number that is already a record is not issued twice
    When I POST JSON {"number":3,"name":"First"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    When I POST JSON {"number":3,"name":"Second"} to "/api/v1/cluster/<kind>"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_number_taken"
    When I send a GET request to "/api/v1/cluster/<kind>/3"
    Then the JSON field "name" should be "First"

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-020
  Scenario Outline: A block of sixteen numbers holds sixteen records
    When I POST JSON {} to "/api/v1/cluster/<kind>" 16 times
    And I POST JSON {} to "/api/v1/cluster/<kind>"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_block_full"
    When I send a GET request to "/api/v1/cluster/<kind>"
    Then the JSON pointer "/<kind>/15/number" should be 15
    And the JSON pointer "/<kind>/16" should be absent

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-021
  Scenario Outline: A rename names the revision it saw and moves it on
    When I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    When I PATCH JSON {"name":"Hall","revision":1} to "/api/v1/cluster/<kind>/2"
    Then the response status should be 200
    And the JSON field "name" should be "Hall"
    And the JSON numeric field "revision" should be 2
    And the JSON pointer "/birth_mark/generation" should be 1
    When I send a GET request to "/api/v1/cluster/<kind>/2"
    Then the JSON field "name" should be "Hall"
    And the JSON numeric field "revision" should be 2

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-022
  Scenario Outline: An edit that names a stale revision changes nothing
    When I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/<kind>"
    And I PATCH JSON {"name":"Hall","revision":1} to "/api/v1/cluster/<kind>/2"
    Then the response status should be 200
    When I PATCH JSON {"name":"Lobby","revision":1} to "/api/v1/cluster/<kind>/2"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_stale_revision"
    When I DELETE JSON {"revision":1} to "/api/v1/cluster/<kind>/2"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_stale_revision"
    When I send a GET request to "/api/v1/cluster/<kind>/2"
    Then the response status should be 200
    And the JSON field "name" should be "Hall"
    And the JSON numeric field "revision" should be 2

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-023
  Scenario Outline: A delete names the revision it saw and removes the record
    When I POST JSON {"number":6,"name":"Corridor"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    When I DELETE JSON {"revision":1} to "/api/v1/cluster/<kind>/6"
    Then the response status should be 204
    When I send a GET request to "/api/v1/cluster/<kind>/6"
    Then the response status should be 404
    When I send a GET request to "/api/v1/cluster/<kind>"
    Then the JSON pointer "/<kind>/0" should be absent

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-024
  Scenario Outline: A number deleted and issued again is another record
    When I POST JSON {"number":5,"name":"Old"} to "/api/v1/cluster/<kind>"
    And I DELETE JSON {"revision":1} to "/api/v1/cluster/<kind>/5"
    Then the response status should be 204
    When I POST JSON {"number":5,"name":"New"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON pointer "/birth_mark/generation" should be 2
    And the JSON numeric field "revision" should be 1

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-025
  Scenario Outline: A freed number is issued again only when no number is left that was never issued
    When I POST JSON {} to "/api/v1/cluster/<kind>"
    And I DELETE JSON {"revision":1} to "/api/v1/cluster/<kind>/0"
    And I POST JSON {} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON numeric field "number" should be 1
    When I POST JSON {} to "/api/v1/cluster/<kind>" 14 times
    And I POST JSON {} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON numeric field "number" should be 0
    And the JSON pointer "/birth_mark/generation" should be 2

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-026
  Scenario Outline: A record this node is not home to is not found, and a number past the last is no record
    When I send a GET request to "/api/v1/cluster/<kind>/7"
    Then the response status should be 404
    When I PATCH JSON {"name":"Hall","revision":1} to "/api/v1/cluster/<kind>/7"
    Then the response status should be 404
    When I DELETE JSON {"revision":1} to "/api/v1/cluster/<kind>/7"
    Then the response status should be 404
    When I send a GET request to "/api/v1/cluster/<kind>/128"
    Then the response status should be 400
    And the JSON error should be "invalid_resource_id"

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-027
  Scenario Outline: The node number cannot change while the node is home to a record
    When I POST JSON {"name":"Corridor"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    When I PATCH JSON {"node_number":1} to "/api/v1/cluster"
    Then the response status should be 409
    And the JSON field "message" should be "cluster_node_number_pinned"
    When I PATCH JSON {"node_number":0,"port":47900} to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON numeric field "port" should be 47900
    When I DELETE JSON {"revision":1} to "/api/v1/cluster/<kind>/0"
    And I PATCH JSON {"node_number":1} to "/api/v1/cluster"
    Then the response status should be 200
    And the JSON numeric field "node_number" should be 1

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-028
  Scenario Outline: The node keeps the generation of a number across a change of node number
    When I POST JSON {"number":3} to "/api/v1/cluster/<kind>"
    And I DELETE JSON {"revision":1} to "/api/v1/cluster/<kind>/3"
    And I PATCH JSON {"node_number":1} to "/api/v1/cluster"
    Then the response status should be 200
    When I POST JSON {"number":19} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON pointer "/birth_mark/generation" should be 2

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-029
  Scenario Outline: Records and the generations of deleted numbers survive a controller restart
    Given in-memory slice persistence is enabled for the host stack
    When I POST JSON {"number":1,"name":"Gone"} to "/api/v1/cluster/<kind>"
    And I DELETE JSON {"revision":1} to "/api/v1/cluster/<kind>/1"
    And I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/<kind>"
    And I PATCH JSON {"name":"Hall","revision":1} to "/api/v1/cluster/<kind>/2"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/cluster/<kind>/2"
    Then the response status should be 200
    And the JSON field "name" should be "Hall"
    And the JSON numeric field "revision" should be 2
    When I POST JSON {"number":1} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    And the JSON pointer "/birth_mark/generation" should be 2

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-030
  Scenario Outline: A standby reads the records and refuses to write them
    Given this controller has stood down
    When I send a GET request to "/api/v1/cluster/<kind>"
    Then the response status should be 200
    When I POST JSON {"name":"Corridor"} to "/api/v1/cluster/<kind>"
    Then the response status should be 409
    And the JSON error should be "controller_standby"
    And the response header "X-Dali2rust-Role" should be "standby"
    When I send a GET request to "/api/v1/cluster/<kind>"
    Then the JSON pointer "/<kind>/0" should be absent

    Examples:
      | kind     |
      | channels |
      | scenes   |

  @id:CLU-031
  Scenario Outline: A record write refuses a body it does not take
    When I POST JSON {"number":2,"name":"Corridor"} to "/api/v1/cluster/<kind>"
    Then the response status should be 201
    When I <method> JSON <body> to "/api/v1/cluster/<path>"
    Then the response status should be <status>
    And the JSON error should be "<error>"
    When I send a GET request to "/api/v1/cluster/<kind>/2"
    Then the JSON field "name" should be "Corridor"

    Examples:
      | kind     | method | path        | body                                     | status | error             |
      | channels | POST   | channels    | {"number":128}                           | 422    | invalid_value     |
      | channels | POST   | channels    | {"number":"one"}                         | 422    | invalid_value     |
      | channels | POST   | channels    | {"name":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"} | 422 | invalid_value |
      | channels | POST   | channels    | {"colour":"red"}                         | 400    | unknown_field     |
      | channels | PATCH  | channels/2  | {"name":"Hall"}                          | 422    | invalid_value     |
      | channels | PATCH  | channels/2  | {"revision":1}                           | 422    | invalid_value     |
      | channels | PATCH  | channels/2  | {"name":"Hall","revision":1,"number":3}  | 422    | unsupported_field |
      | channels | DELETE | channels/2  | {}                                       | 422    | invalid_value     |
      | scenes   | PATCH  | scenes/2    | {"name":"Hall","revision":-1}            | 422    | invalid_value     |
      | scenes   | DELETE | scenes/2    | {"revision":"1"}                         | 422    | invalid_value     |
