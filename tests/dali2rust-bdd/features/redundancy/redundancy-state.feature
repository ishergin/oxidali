@stage-R17
Feature: Redundancy — state and the planned handover

  What happened, as opposed to what was configured. Read-only and served in
  BOTH roles: a standby that would not answer a read is a standby nobody can
  diagnose during the incident it exists for.

  @id:RED-020
  Scenario: The state resource names the role, the lease and the replication tally
    When I send a GET request to "/api/v1/redundancy"
    Then the response status should be 200
    And the JSON boolean field "active" should be true
    And the JSON field "role" should be "primary"
    And the JSON pointer "/probes/published" should be 0
    And the JSON pointer "/replication/passes" should be 0
    And the JSON pointer "/replication/peer_unreachable" should be 0

  @id:RED-021
  Scenario: Every response says which controller answered it
    When I send a GET request to "/api/v1/health"
    Then the response status should be 200
    And the response header "X-Dali2rust-Role" should be "active"

  @id:RED-022
  Scenario: A switchover with no peer address fails loudly instead of broadcasting
    When I send a POST request to "/api/v1/redundancy/switchover" with empty body
    Then the response status should be 422
    And the JSON error should be "peer_not_configured"

  @id:RED-023
  Scenario: A passive controller refuses a wire-bound write and says why
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given this controller has stood down
    And a successful target-state script for level 180 on short address 0
    When I PUT JSON {"power":"on","level":180} to "/api/v1/adapters/0/physical-devices/0/target-state"
    Then the response status should be 409
    And the JSON error should be "controller_standby"
    And the response header "Retry-After" should be "1"
    And the response header "X-Dali2rust-Role" should be "standby"
    When I send a GET request to "/api/v1/diagnostics"
    Then the JSON pointer "/dali_worker/tx_suppressed_passive" should be 1

  @id:RED-024
  Scenario: A passive controller still answers reads and still takes settings
    Given this controller has stood down
    When I send a GET request to "/api/v1/redundancy"
    Then the response status should be 200
    And the JSON boolean field "active" should be false
    When I PATCH JSON {"probe_interval_ms":500} to "/api/v1/settings/redundancy"
    Then the response status should be 200

  @id:RED-025
  Scenario: Only the controller holding the bus may give it away
    Given this controller has stood down
    When I send a POST request to "/api/v1/redundancy/switchover" with empty body
    Then the response status should be 409
    And the JSON error should be "not_the_active_controller"

  @id:RED-026
  Scenario: The first settle after enabling is a change of hands, and it is recorded
    When I PATCH JSON {"enabled":true,"role":"primary","boot_listen_ms":0} to "/api/v1/settings/redundancy"
    Then the response status should be 200
    And the JSON pointer "/transitions/0/reason" at "/api/v1/redundancy" should eventually be "boot"
    When I send a GET request to "/api/v1/redundancy"
    Then the JSON pointer "/transitions/0/now_active" should be true
    And the JSON pointer "/transitions/0/missed_probes" should be 0

  @id:RED-027
  Scenario: A planned switchover is recorded as a handover on the side that gave the bus away
    When I PATCH JSON {"enabled":true,"role":"primary","boot_listen_ms":0,"peer_device_short_address":61} to "/api/v1/settings/redundancy"
    Then the response status should be 200
    And the JSON pointer "/transitions/0/reason" at "/api/v1/redundancy" should eventually be "boot"
    When I send a POST request to "/api/v1/redundancy/switchover" with empty body
    Then the response status should be 200
    And the JSON pointer "/transitions/0/reason" at "/api/v1/redundancy" should eventually be "handover"
    When I send a GET request to "/api/v1/redundancy"
    Then the JSON boolean field "active" should be false
    And the JSON pointer "/transitions/0/now_active" should be false
    And the JSON pointer "/transitions/1/reason" should be "boot"

  @id:RED-028
  Scenario: A standby's arbitration probe announces itself at priority 5
    When I PATCH JSON {"enabled":true,"role":"standby","boot_listen_ms":0} to "/api/v1/settings/redundancy"
    Then the response status should be 200
    And the standby's arbitration probe should eventually be sent at priority 5
