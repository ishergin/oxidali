@stage-R17
Feature: Redundancy — the wake that starts replication early

  A passive controller pulls its peer's configuration on a period. When the
  active controller saves a slice that transfers, it tells its peer so with a
  request that carries no data: the passive one only starts the same manifest
  compare it would have run at the period, and a lost wake costs no more than
  the period itself.

  @id:RED-031
  Scenario: A wake from the peer starts the manifest compare at once, not at the period
    Given a peer controller is listening
    And this controller has stood down
    When I PATCH JSON {"enabled":true,"role":"primary"} to "/api/v1/settings/redundancy" naming the peer
    Then the response status should be 200
    And the peer should eventually have received 1 "GET /api/v1/config/slices"
    When I send a POST request to "/api/v1/redundancy/config-changed" with empty body
    Then the response status should be 204
    And the peer should eventually have received 2 "GET /api/v1/config/slices"
    And the JSON pointer "/replication/wakes_received" at "/api/v1/redundancy" should eventually be "1"

  @id:RED-032
  Scenario: A saved change that transfers wakes the peer once, and one that never transfers does not
    Given in-memory slice persistence is enabled for the host stack
    And a peer controller is listening
    When I export "/api/v1/config/slices/home_assistant_settings" once it is stored and keep the body
    And I note how many slices this controller has saved
    And I PATCH JSON {"enabled":true,"role":"primary","boot_listen_ms":0} to "/api/v1/settings/redundancy" naming the peer
    Then the response status should be 200
    And this controller should eventually have saved more slices than noted
    And the peer should have received 0 "POST /api/v1/redundancy/config-changed" and no more for 1000 ms
    When I PATCH JSON {"interval_ms":5001} to "/api/v1/settings/poller"
    Then the response status should be 200
    And the peer should eventually have received 1 "POST /api/v1/redundancy/config-changed"
    And the peer should have received 1 "POST /api/v1/redundancy/config-changed" and no more for 1000 ms
    And the JSON pointer "/replication/wakes_sent" at "/api/v1/redundancy" should eventually be "1"

  @id:RED-033
  Scenario: A passive controller never wakes its peer, whatever it saves
    Given in-memory slice persistence is enabled for the host stack
    And a peer controller is listening
    And this controller has stood down
    When I PATCH JSON {"enabled":true,"role":"primary"} to "/api/v1/settings/redundancy" naming the peer
    Then the response status should be 200
    When I PATCH JSON {"interval_ms":5001} to "/api/v1/settings/poller"
    Then the response status should be 200
    When I export "/api/v1/config/slices/poller_settings" once it is stored and keep the body
    Then the peer should have received 0 "POST /api/v1/redundancy/config-changed" and no more for 1000 ms
