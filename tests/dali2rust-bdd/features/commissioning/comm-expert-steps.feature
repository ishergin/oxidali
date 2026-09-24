@stage-R15
Feature: Commissioning expert steps

  @id:COMM-080
  Scenario: Initialise step reaches the wire and creates no operation
    Given an expert step script for initialise unaddressed
    When I POST JSON {"scope":"unaddressed"} to "/api/v1/adapters/0/commissioning/steps/initialise"
    Then the response status should be 200
    And all scripted DALI exchanges should be consumed without errors
    When I send a GET request to "/api/v1/operations"
    Then the response status should be 200
    And the operations list should contain exactly 0 operations

  @id:COMM-081
  Scenario: Search-address step stages a typed 24-bit address on the wire
    Given an expert step script for search address 1193046
    When I POST JSON {"search_address":1193046} to "/api/v1/adapters/0/commissioning/steps/search-address"
    Then the response status should be 200
    And all scripted DALI exchanges should be consumed without errors

  @id:COMM-082
  Scenario: Compare step returns a typed match result
    Given an expert step script for compare answering yes
    When I POST JSON {} to "/api/v1/adapters/0/commissioning/steps/compare"
    Then the response status should be 200
    And the commissioning step result match should be true

  @id:COMM-083
  Scenario: Expert step rejects an unknown step name
    When I POST JSON {} to "/api/v1/adapters/0/commissioning/steps/foo"
    Then the response status should be 404
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-084
  Scenario: Program-short-address step validates the DALI short-address range
    When I POST JSON {"short_address":99} to "/api/v1/adapters/0/commissioning/steps/program-short-address"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-087
  Scenario: Initialise step rejects a scope outside the documented enum
    When I POST JSON {"scope":"everything"} to "/api/v1/adapters/0/commissioning/steps/initialise"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-088
  Scenario: Search-address step rejects a value that does not fit 24 bits
    When I POST JSON {"search_address":16777216} to "/api/v1/adapters/0/commissioning/steps/search-address"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-089
  Scenario: Query-short-address step returns a typed short address
    Given an expert step script for query short address answering 17
    When I POST JSON {} to "/api/v1/adapters/0/commissioning/steps/query-short-address"
    Then the response status should be 200
    And the commissioning step result short_address should be 17

  @id:COMM-091
  Scenario: Query-short-address reports silence as absent, not as address 0
    Given an expert step script for query short address answering nothing
    When I POST JSON {} to "/api/v1/adapters/0/commissioning/steps/query-short-address"
    Then the response status should be 200
    And the commissioning step result short_address should be absent

  @id:COMM-090
  Scenario: Expert step reports a request-scoped confirmation timeout
    Given a bus with confirmation timeout of 500 milliseconds
    And the DALI transport blocks indefinitely
    When I POST JSON {} to "/api/v1/adapters/0/commissioning/steps/randomise"
    Then the response status should be 504

  @id:COMM-093
  Scenario: Compare reads a violating backward frame as YES
    Given an expert step script for compare answering with a violating frame
    When I POST JSON {} to "/api/v1/adapters/0/commissioning/steps/compare"
    Then the response status should be 200
    And the commissioning step result match should be true

  @id:COMM-094
  Scenario: Query-short-address reports several gear answering as its own outcome
    Given an expert step script for query short address answering with a violating frame
    When I POST JSON {} to "/api/v1/adapters/0/commissioning/steps/query-short-address"
    Then the response status should be 200
    And the commissioning step result short_address should be absent
    And the commissioning step result answer should be "multiple"

  @id:COMM-095
  Scenario: Query-short-address reports MASK as unaddressed, not as address 63
    Given an expert step script for query short address answering mask
    When I POST JSON {} to "/api/v1/adapters/0/commissioning/steps/query-short-address"
    Then the response status should be 200
    And the commissioning step result short_address should be absent
    And the commissioning step result answer should be "unaddressed"

