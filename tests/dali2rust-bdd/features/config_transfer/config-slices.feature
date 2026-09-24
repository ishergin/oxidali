@stage-X1
Feature: Configuration slices — export and import

  The configuration a standby cannot learn from the wire. The sniffer carries
  state — levels, colour, who is on the segment — and carries nothing that
  makes a controller more than a relay: names, virtual-lamp bindings, groups,
  scenes, schedules, rules. This resource is how those cross, and it is the
  backup the product owed itself regardless.

  Bytes are the stored postcard envelope, not JSON: re-encoding would make the
  export a second serialisation to keep in agreement with the first.

  Background:
    Given in-memory slice persistence is enabled for the host stack

  @id:CFG-001
  Scenario: The manifest names each transferable slice with a length and a CRC
    When I send a GET request to "/api/v1/config/slices"
    Then the response status should be 200
    And the JSON pointer "/0/name" should be present
    And the JSON pointer "/0/crc32" should be present

  @id:CFG-002
  Scenario: A slice exports as an opaque blob once something has written it
    When I PATCH JSON {"interval_ms":60000} to "/api/v1/settings/poller"
    Then the response status should be 200
    When I send a GET request to "/api/v1/config/slices/poller_settings"
    Then the response status should be 200
    And the response content type should be "application/octet-stream"

  @id:CFG-003
  Scenario: A slice this build does not know is not found
    When I send a GET request to "/api/v1/config/slices/no_such_slice"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:CFG-004
  Scenario: A slice that is deliberately not transferable is not found either
    When I send a GET request to "/api/v1/config/slices/dali_settings"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:CFG-005
  Scenario: An empty body is refused rather than stored as an empty slice
    When I send a PUT request to "/api/v1/config/slices/poller_settings"
    Then the response status should be 422
    And the JSON error should be "empty_slice"

  @id:CFG-006
  Scenario: An import aimed at a slice this build does not carry is not found
    When I PUT JSON {"anything":1} to "/api/v1/config/slices/no_such_slice"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:CFG-007
  Scenario: The manifest carries the timezone first and the Home Assistant settings with the rest
    When I send a GET request to "/api/v1/config/slices"
    Then the response status should be 200
    And the JSON pointer "/0/name" should be "settings"
    And the JSON pointer "/4/name" should be "home_assistant_settings"

  @id:CFG-008
  Scenario: An exported Home Assistant slice carries the broker and its credentials
    When I PATCH JSON {"broker_host":"broker.local","broker_password":"hunter2"} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    And the JSON boolean field "broker_password_set" should be true
    When I send a GET request to "/api/v1/config/slices/home_assistant_settings"
    Then the response status should be 200
    And the response body should contain "broker.local"
    And the response body should contain "hunter2"

  @id:CFG-009
  Scenario: An imported Home Assistant slice is the whole truth, credentials included
    When I PATCH JSON {"broker_host":"broker.local","broker_password":"hunter2"} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    When I export "/api/v1/config/slices/home_assistant_settings" and keep the body
    And I PATCH JSON {"broker_host":"other.local"} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    When I PUT the kept body to "/api/v1/config/slices/home_assistant_settings"
    Then the response status should be 200
    And the JSON pointer "/broker_host" at "/api/v1/settings/home-assistant" should eventually be "broker.local"
    And the JSON pointer "/broker_password_set" at "/api/v1/settings/home-assistant" should eventually be "true"

  @id:CFG-010
  Scenario: An imported timezone takes effect without a reboot
    When I PUT JSON {"timezone":"EST5"} to "/api/v1/time"
    Then the response status should be 200
    When I export "/api/v1/config/slices/settings" and keep the body
    And I PUT JSON {"timezone":"UTC0"} to "/api/v1/time"
    Then the response status should be 200
    And the JSON pointer "/timezone" at "/api/v1/time" should eventually be "UTC0"
    When I PUT the kept body to "/api/v1/config/slices/settings"
    Then the response status should be 200
    And the JSON pointer "/timezone" at "/api/v1/time" should eventually be "EST5"

  @id:CFG-011
  Scenario: Re-importing a rules bank leaves the live document intact
    When I PUT JSON {"base_revision":0,"source":"# импорт\nrule \"ночь\" {\n  when at 23:00\n  do broadcast.off()\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And the JSON pointer "/revision" at "/api/v1/rules" should eventually be "1"
    When I export "/api/v1/config/slices/rules_b0" and keep the body
    And I PUT the kept body to "/api/v1/config/slices/rules_b0"
    Then the response status should be 200
    And the JSON pointer "/rule_count" at "/api/v1/rules" should eventually be "1"
    And the JSON pointer "/revision" at "/api/v1/rules" should eventually be "1"
    When I send a GET request to "/api/v1/rules"
    Then the response body should contain "# импорт"
    And the JSON pointer "/diagnostic" should be null
