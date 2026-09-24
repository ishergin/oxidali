@stage-R11
Feature: Settings — Home Assistant

  @id:SET-HA-001
  Scenario: GET never exposes the broker password, only whether one is set
    When I PATCH JSON {"broker_username":"ha-user","broker_password":"secret"} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    When I send a GET request to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    And the JSON field "broker_username" should be "ha-user"
    And the JSON boolean field "broker_password_set" should be true
    And the JSON field "broker_password" should be absent

  @id:SET-HA-002
  Scenario: GET returns documented defaults before any PATCH
    When I send a GET request to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be false
    And the JSON numeric field "broker_port" should be 1883
    And the JSON field "discovery_prefix" should be "homeassistant"
    And the JSON field "state_topic_prefix" should be "dali"
    And the JSON numeric field "publish_qos" should be 1
    And the JSON boolean field "retain_state" should be true
    And the JSON boolean field "retain_discovery" should be true
    And the JSON boolean field "broker_password_set" should be false

  @id:SET-HA-010
  Scenario: One PATCH spanning three commands applies as a whole
    When I PATCH JSON {"enabled":true,"broker_host":"mqtt.example.org","broker_username":"ha-user","broker_password":"secret","controller_id":"ctl1"} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    When I send a GET request to "/api/v1/settings/home-assistant"
    Then the JSON boolean field "enabled" should be true
    And the JSON field "broker_host" should be "mqtt.example.org"
    And the JSON field "broker_username" should be "ha-user"
    And the JSON boolean field "broker_password_set" should be true
    And the JSON field "controller_id" should be "ctl1"

  @id:SET-HA-011
  Scenario: PATCH refuses a derived field as unsupported, not unknown
    When I PATCH JSON {"broker_password_set":true} to "/api/v1/settings/home-assistant"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

  @id:SET-HA-012
  Scenario: PATCH refuses the derived broker URL as unsupported
    When I PATCH JSON {"broker_url_view":"mqtt://x:1883"} to "/api/v1/settings/home-assistant"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

  @id:SET-HA-013
  Scenario: PATCH rejects a publish_qos the bridge cannot honour
    When I PATCH JSON {"publish_qos":5} to "/api/v1/settings/home-assistant"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SET-HA-014
  Scenario: PATCH rejects a field the resource does not have
    When I PATCH JSON {"broker_tls":true} to "/api/v1/settings/home-assistant"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:SET-HA-015
  Scenario: PATCH rejects an empty controller_id
    When I PATCH JSON {"controller_id":""} to "/api/v1/settings/home-assistant"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SET-HA-016
  Scenario: PATCH rejects a controller_id that would build an unroutable topic
    When I PATCH JSON {"controller_id":"my controller"} to "/api/v1/settings/home-assistant"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SET-HA-017
  Scenario: PATCH rejects a password longer than the wire can carry
    When I PATCH JSON {"broker_password":"0123456789012345678901234567890123456789012345678"} to "/api/v1/settings/home-assistant"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:SET-HA-018
  Scenario: The derived broker URL follows host and port
    When I PATCH JSON {"broker_host":"mqtt.example.org","broker_port":8883} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    And the JSON field "broker_url_view" should be "mqtt://mqtt.example.org:8883"

  @id:SET-HA-019
  Scenario: Home Assistant settings survive a controller restart
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"broker_host":"mqtt.example.org","controller_id":"ctl1"} to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/settings/home-assistant"
    Then the response status should be 200
    And the JSON field "broker_host" should be "mqtt.example.org"
    And the JSON field "controller_id" should be "ctl1"

  @id:SET-HA-020
  Scenario: A discovery republish is accepted as an operation
    Given the Home Assistant bridge is enabled with controller id "ctl1"
    When I POST JSON {} to "/api/v1/settings/home-assistant/discovery-publish"
    Then the response status should be 202
    And the JSON field "type" should be "ha_discovery_publish"
    And the JSON field "operation_id" should be non-empty
    And the last operation eventually succeeds
