@stage-R17
Feature: Redundancy — settings

  What the operator configured, as opposed to what happened. Off by default:
  until `enabled` is true this controller behaves exactly as a single one
  always has, and nothing probes the segment.

  The two refusals below are the substance of the resource. A role that does
  not parse is NOT rounded down to `primary` — two units both rounding down
  both drive the bus — and a `peer_url` is never truncated to fit, because a
  host name cut at sixty-four characters is a DIFFERENT host, and a standby
  would pull an installation's configuration from whatever answers there.

  @id:RED-001
  Scenario: GET returns the documented defaults before any PATCH
    When I send a GET request to "/api/v1/settings/redundancy"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be false
    And the JSON field "role" should be "primary"
    And the JSON numeric field "probe_interval_ms" should be 400
    And the JSON numeric field "takeover_after_missed" should be 2
    And the JSON field "peer_url" should be ""

  @id:RED-002
  Scenario: PATCH switches this unit to a standby and the change survives a re-read
    When I PATCH JSON {"enabled":true,"role":"standby","probe_interval_ms":250} to "/api/v1/settings/redundancy"
    Then the response status should be 200
    And the JSON field "role" should be "standby"
    When I send a GET request to "/api/v1/settings/redundancy"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be true
    And the JSON field "role" should be "standby"
    And the JSON numeric field "probe_interval_ms" should be 250

  @id:RED-003
  Scenario: An unknown key is refused rather than ignored
    When I PATCH JSON {"nope":1} to "/api/v1/settings/redundancy"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:RED-004
  Scenario: An unreadable role is refused, never rounded down to primary
    When I PATCH JSON {"role":"whatever"} to "/api/v1/settings/redundancy"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
    When I send a GET request to "/api/v1/settings/redundancy"
    Then the JSON field "role" should be "primary"

  @id:RED-005
  Scenario: A probe interval below the floor is refused
    When I PATCH JSON {"probe_interval_ms":100} to "/api/v1/settings/redundancy"
    Then the response status should be 422
    And the JSON error should be "out_of_range"

  @id:RED-006
  Scenario: A peer URL that is not http://host[:port] is refused
    When I PATCH JSON {"peer_url":"https://peer.example"} to "/api/v1/settings/redundancy"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:RED-007
  Scenario: A peer URL too long to store is refused rather than truncated
    When I PATCH JSON {"peer_url":"http://a-very-long-standby-controller-name.example.internal.lan:8080"} to "/api/v1/settings/redundancy"
    Then the response status should be 422
    And the JSON error should be "out_of_range"

  @id:RED-008
  Scenario: A peer URL is stored and cleared by the empty string
    When I PATCH JSON {"peer_url":"http://192.168.1.11"} to "/api/v1/settings/redundancy"
    Then the response status should be 200
    And the JSON field "peer_url" should be "http://192.168.1.11"
    When I PATCH JSON {"peer_url":""} to "/api/v1/settings/redundancy"
    Then the response status should be 200
    And the JSON field "peer_url" should be ""
