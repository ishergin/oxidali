@stage-I10
Feature: Rules and the controller's own records: log lines and named counters

  `log("…")` writes to the controller log under the target `rules`, so the
  line reaches the log ring and the `logs` WebSocket channel as any firmware
  line does, named after the rule that wrote it. `stat("name").count()`
  counts per name in the rules block of `/api/v1/stats`; the counts live in
  RAM, a name starts at 0 when a document brings it, and a new document keeps
  the counts of the names it keeps.

  @id:RULE-067
  Scenario: A rule's log line reaches the logs channel under the rules target, named after its rule
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "logs" at level "info"
    When I PUT JSON {"base_revision":0,"source":"rule \"night\" {\n  when http trigger\n  do log(\"lights out\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I POST JSON {} to "/api/v1/rules/night/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the WebSocket client should receive a "rules" log line reading "night: lights out"

  @id:RULE-068
  Scenario: Named counters count per name, and a new document keeps the counts of the names it keeps
    When I PUT JSON {"base_revision":0,"source":"rule \"press\" {\n  when http trigger\n  do stat(\"presses\").count()\n}\nrule \"tap\" {\n  when http trigger\n  do stat(\"presses\").count()\n     stat(\"taps\").count()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the stats eventually list the rule counter "presses" at 0
    And the stats list the rule counter "taps" at 0
    When I POST JSON {} to "/api/v1/rules/press/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I POST JSON {} to "/api/v1/rules/tap/run"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the stats eventually list the rule counter "presses" at 2
    And the stats list the rule counter "taps" at 1
    When I PUT JSON {"base_revision":1,"source":"rule \"press\" {\n  when http trigger\n  do stat(\"presses\").count()\n}\nrule \"hold\" {\n  when http trigger\n  do stat(\"holds\").count()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the stats eventually list the rule counter "holds" at 0
    And the stats list the rule counter "presses" at 2
    And the stats do not list the rule counter "taps"
