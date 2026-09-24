@stage-I10
Feature: The rules document: parse, store, read back byte-for-byte

  The text is the canon (ADR-016 A2): the device stores the operator's own
  bytes, comments included, and there is no printer to regenerate them. REST
  treats the document as ONE resource — GET returns the whole source, PUT
  replaces it as a staged atomic write behind 202 + operation, POST /parse
  validates without touching anything, and PATCH /{name} flips one enabled
  bit. Since I10-B the engine is real, so /run of an unknown rule is the 404
  its name deserves — the 501 this file shipped with died with the stub.

  @id:RULE-001
  Scenario: Parse validates a document without storing anything
    When I POST JSON {"source":"rule \"свет\" {\n  when every 5m\n  do log(\"tick\")\n}\n"} to "/api/v1/rules/parse"
    Then the response status should be 200
    And the JSON pointer "/ok" should be true
    And the JSON pointer "/rules/0/name" should be "свет"
    When I send a GET request to "/api/v1/rules"
    Then the JSON pointer "/revision" should be 0

  @id:RULE-002
  Scenario: A parse error names its line and column
    When I POST JSON {"source":"rule \"x\" {\n  when every 5m\n  do explode()\n}\n"} to "/api/v1/rules/parse"
    Then the response status should be 400
    And the JSON pointer "/error" should be "parse_error"
    And the JSON pointer "/line" should be 3

  @id:RULE-003
  Scenario: PUT stores the document and GET returns the bytes verbatim
    When I PUT JSON {"base_revision":0,"source":"# ночной свет\nrule \"ночь\" {\n  when at 23:00\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/rules"
    Then the response status should be 200
    And the JSON pointer "/revision" should be 1
    And the JSON pointer "/rule_count" should be 1
    And the response body should contain "# ночной свет"

  @id:RULE-004
  Scenario: The typed projection is served beside the canon, never instead of it
    When I PUT JSON {"base_revision":0,"source":"rule \"ночь\" {\n  when at 23:00\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I send a GET request to "/api/v1/rules?format=json"
    Then the response status should be 200
    And the JSON pointer "/rules/rules/0/name" should be "ночь"
    And the JSON pointer "/rules/rules/0/enabled" should be true

  @id:RULE-005
  Scenario: A stale base revision is refused with a conflict, not merged
    When I PUT JSON {"base_revision":0,"source":"rule \"a\" {\n  when every 5m\n  do log(\"a\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PUT JSON {"base_revision":0,"source":"rule \"b\" {\n  when every 5m\n  do log(\"b\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 409
    And the response body should contain "rule_set_conflict"

  @id:RULE-006
  Scenario: PATCH flips one rule's enabled bit without touching the source
    When I PUT JSON {"base_revision":0,"source":"rule \"night\" {\n  when at 23:00\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"enabled":false} to "/api/v1/rules/night"
    Then the response status should be 200
    When I send a GET request to "/api/v1/rules?format=json"
    Then the JSON pointer "/rules/rules/0/enabled" should be false
    And the JSON pointer "/revision" should be 2

  @id:RULE-007
  Scenario: PATCH of an unknown rule is a 404 by name
    When I PATCH JSON {"enabled":false} to "/api/v1/rules/ghost"
    Then the response status should be 404
    And the response body should contain "rule_not_found"

  @id:RULE-008
  Scenario: Run of an unknown rule is refused by name, not accepted into limbo
    When I POST JSON {} to "/api/v1/rules/night/run"
    Then the response status should be 404
    And the response body should contain "rule_not_found"

  @id:RULE-009
  Scenario: A document over the three-bank ceiling is refused before staging
    When I PUT an oversized rules document
    Then the response status should be 413
    And the response body should contain "rule_document_too_large"
