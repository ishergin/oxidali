@stage-R10
Feature: Settings — Poller

  @id:SET-POL-001
  Scenario: GET returns the DTO before any PATCH
    When I send a GET request to "/api/v1/settings/poller"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be false

  @id:SET-POL-002
  Scenario: GET returns documented defaults before any PATCH
    When I send a GET request to "/api/v1/settings/poller"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be false
    And the JSON numeric field "interval_ms" should be 5000
    And the JSON field "attribute_groups_default" should equal ["runtime_status"]
    And the JSON boolean field "include_dt8_color" should be true
    And the JSON boolean field "skip_unbound_virtual_lamps" should be true
    And the JSON boolean field "include_energy" should be false
    And the JSON boolean field "include_diagnostics" should be false

  @id:SET-POL-010
  Scenario: PATCH applies the requested change and returns it
    When I PATCH JSON {"interval_ms":60000} to "/api/v1/settings/poller"
    Then the response status should be 200
    And the JSON numeric field "interval_ms" should be 60000

  @id:SET-POL-011
  Scenario: PATCH rejects interval_ms below the floor
    When I PATCH JSON {"interval_ms":100} to "/api/v1/settings/poller"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SET-POL-012
  Scenario: PATCH rejects an unknown attribute group
    When I PATCH JSON {"attribute_groups_default":["nonsense"]} to "/api/v1/settings/poller"
    Then the response status should be 422
    And the JSON error should be "invalid_enum"

  @id:SET-POL-013
  Scenario: PATCH rejects the retired max_concurrent field
    When I PATCH JSON {"max_concurrent":2} to "/api/v1/settings/poller"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:SET-POL-014
  Scenario: PATCH round-trips through a subsequent GET
    When I PATCH JSON {"interval_ms":60000,"enabled":true} to "/api/v1/settings/poller"
    Then the response status should be 200
    When I send a GET request to "/api/v1/settings/poller"
    Then the JSON numeric field "interval_ms" should be 60000
    And the JSON boolean field "enabled" should be true

  @id:SET-POL-015
  Scenario: PATCH rejects unknown fields including backoff knobs
    When I PATCH JSON {"backoff_ms":1000} to "/api/v1/settings/poller"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:SET-POL-016
  Scenario: PATCH rejects an empty attribute group list
    When I PATCH JSON {"attribute_groups_default":[]} to "/api/v1/settings/poller"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SET-POL-017
  Scenario: Poller settings survive a controller restart
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"interval_ms":60000} to "/api/v1/settings/poller"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/settings/poller"
    Then the response status should be 200
    And the JSON numeric field "interval_ms" should be 60000

  @id:SET-POL-018
  Scenario: The two bank-read switches are independent
    When I PATCH JSON {"include_energy":true} to "/api/v1/settings/poller"
    Then the response status should be 200
    And the JSON boolean field "include_energy" should be true
    And the JSON boolean field "include_diagnostics" should be false
    When I PATCH JSON {"include_diagnostics":true} to "/api/v1/settings/poller"
    Then the response status should be 200
    And the JSON boolean field "include_energy" should be true
    And the JSON boolean field "include_diagnostics" should be true

  @id:SET-POL-019
  Scenario: Bank-read switches survive a controller restart
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"include_diagnostics":true} to "/api/v1/settings/poller"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/settings/poller"
    Then the response status should be 200
    And the JSON boolean field "include_diagnostics" should be true
