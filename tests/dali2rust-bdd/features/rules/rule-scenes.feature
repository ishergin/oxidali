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
