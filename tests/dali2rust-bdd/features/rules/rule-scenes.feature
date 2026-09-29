@stage-I10
Feature: Rules and scenes: a recall is one frame and a fact once it has happened

  `scene(N).recall(target)` is one native GO TO SCENE on a group, on broadcast
  or on the short address a lamp is bound to. `scene(N) recalled` wakes on a
  GO TO SCENE that happened, never on one that failed. The positive controls
  order the proof: the rules funnel takes one publisher's events in order, so
  once a later recall has fired its rule, the verdict on every earlier recall
  is in.

  @id:RULE-030
  Scenario: A recall that failed on the wire wakes no scene rule, one that happened does
    When I PUT JSON {"base_revision":0,"source":"rule \"after-3\" cooldown 0ms {\n  when scene(3) recalled\n  do log(\"3\")\n}\nrule \"after-4\" cooldown 0ms {\n  when scene(4) recalled\n  do log(\"4\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport fails the next broadcast go-to-scene 3 frame
    When I send a POST request to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 503
    When I send a POST request to "/api/v1/adapters/0/scenes/4/recall"
    Then the response status should be 200
    And the rule "after-4" eventually has fired 1 time
    And the rule "after-3" should have fired 0 times
    When I send a POST request to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 200
    And the rule "after-3" eventually has fired 1 time

  @id:RULE-031
  Scenario: A rule recalls a scene on one lamp with one GO TO SCENE to its short address
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    When I PUT JSON {"base_revision":0,"source":"rule \"one\" {\n  when http trigger\n  do scene(3).recall(lamp(1))\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {} to "/api/v1/rules/one/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the virtual lamp 1 runtime level should eventually be 100
    And the virtual lamp 1 last_dapc_source should be "scene"
    And the DALI mock transport should have sent only a short 0 go-to-scene 3 frame

  @id:RULE-032
  Scenario: A recall on a lamp with no binding reaches no wire and is counted as a failure
    When I PUT JSON {"base_revision":0,"source":"rule \"nowhere\" {\n  when http trigger\n  do scene(3).recall(lamp(2))\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When I POST JSON {} to "/api/v1/rules/nowhere/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And within 3 seconds the stats pointer "/dali/errors_total" reaches 1
    And the mock transport should have sent no frames

  @id:RULE-033
  Scenario: A recall commits under the source that asked for it
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And adapter 0 scene 4 desired row for virtual lamp 1 has level 150
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    Given adapter 0 scene 4 write for short 0 level 150 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/4/apply"
    Then the last operation eventually succeeds
    When I PUT JSON {"base_revision":0,"source":"rule \"evening\" {\n  when http trigger\n  do scene(4).recall(broadcast)\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a POST request to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 200
    And virtual lamp 1 on adapter 0 should eventually report level 100 from value_source "api"
    When I POST JSON {} to "/api/v1/rules/evening/run"
    Then the response status should be 202
    And virtual lamp 1 on adapter 0 should eventually report level 150 from value_source "rules"

  @id:RULE-034
  Scenario: A foreign GO TO SCENE wakes the scene rule once, and so does one recall of ours
    When I PUT JSON {"base_revision":0,"source":"rule \"after-3\" cooldown 0ms {\n  when scene(3) recalled\n  do log(\"3\")\n}\nrule \"after-4\" cooldown 0ms {\n  when scene(4) recalled\n  do log(\"4\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When a foreign broadcast recall of scene 3 is observed on the bus
    Then the rule "after-3" eventually has fired 1 time
    When I send a POST request to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 200
    And the rule "after-3" eventually has fired 2 times
    When I send a POST request to "/api/v1/adapters/0/scenes/4/recall"
    Then the response status should be 200
    And the rule "after-4" eventually has fired 1 time
    And the rule "after-3" should have fired 2 times

  @id:RULE-035
  Scenario: A foreign recall on a group or one short address wakes the scene rule, one to gear without an address does not
    When I PUT JSON {"base_revision":0,"source":"rule \"after-5\" cooldown 0ms {\n  when scene(5) recalled\n  do log(\"5\")\n}\nrule \"after-6\" cooldown 0ms {\n  when scene(6) recalled\n  do log(\"6\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When a foreign unaddressed recall of scene 5 is observed on the bus
    And a foreign broadcast recall of scene 6 is observed on the bus
    Then the rule "after-6" eventually has fired 1 time
    And the rule "after-5" should have fired 0 times
    When a foreign group 2 recall of scene 5 is observed on the bus
    Then the rule "after-5" eventually has fired 1 time
    When a foreign short address 7 recall of scene 5 is observed on the bus
    Then the rule "after-5" eventually has fired 2 times

  @id:RULE-036
  Scenario: A foreign recall is projected once, under the sniffer's name
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    Given the DALI mock transport trace is cleared
    When a foreign broadcast recall of scene 3 is observed on the bus
    Then virtual lamp 1 on adapter 0 should eventually report level 100 from value_source "sniffer"
    When a foreign DAPC frame for short address 0 level 90 is observed on the bus
    Then the virtual lamp 1 runtime level should eventually be 90
    And the diagnostics projector counter "scene_expansions" should be 1
    And the DALI mock transport should have received 0 forward frame
