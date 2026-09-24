@stage-R15
Feature: Commissioning identify / locate

  @id:COMM-030
  Scenario: Identify accepts the request and creates a tracked operation
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an identify script for short address 0
    When I POST JSON {"short_address":0} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 202
    When I send a GET request to "/api/v1/operations"
    Then the response status should be 200
    And the operations list should contain a key starting with "comm-ident-"

  @id:COMM-032
  Scenario: Successful identify reports the located device and its mechanism
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an identify script for short address 0
    When I POST JSON {"short_address":0} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the operation result short_address should be 0
    And the operation result identify_mechanism should be "identify_device"

  @id:COMM-033
  Scenario: Identify rejects an unknown physical device
    When I POST JSON {"short_address":9} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 404
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-034
  Scenario: Identify is executed by the DALI worker through the transport
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an identify script for short address 0
    When I POST JSON {"short_address":0} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors

  @id:COMM-035
  Scenario: Identify rejects a short address outside 0..63
    When I POST JSON {"short_address":64} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 422
    And the DALI mock transport should have received 0 forward frames

  @id:COMM-036
  Scenario: Identify refuses a duration, because the window is the gear's
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    When I POST JSON {"short_address":0,"duration_ms":5000} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

  @id:COMM-038
  Scenario: Identify puts exactly one send-twice IDENTIFY DEVICE pair on the wire
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given an identify script for short address 0
    When I POST JSON {"short_address":0} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the transport should observe an identify device pair for short address 0
    And all scripted DALI exchanges should be consumed without errors

  @id:COMM-092
  Scenario: Identify sends no level command at all
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a successful target-state script for level 128 on short address 0
    When I PUT JSON {"power":"on","level":128} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 200
    Given an identify script for short address 0
    When I POST JSON {"short_address":0} to "/api/v1/adapters/0/commissioning/identify"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the identify sequence for short address 0 should contain no level command
    And all scripted DALI exchanges should be consumed without errors
