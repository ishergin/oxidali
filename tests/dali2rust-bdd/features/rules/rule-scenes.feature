@stage-I10
Feature: Rules and scenes: a recall is a fact once it has happened

  `scene(N) recalled` wakes on a GO TO SCENE that happened, never on one that
  failed. The positive controls order the proof: the rules funnel takes one
  publisher's events in order, so once a later recall has fired its rule, the
  verdict on every earlier recall is in.

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
