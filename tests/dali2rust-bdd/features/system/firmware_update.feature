@stage-X1
Feature: Firmware update over the network
  As an operator who cannot reach the controller with a cable
  I want it to fetch a new image over the network and boot into it
  So that a fix does not require a site visit

  @id:SYS-242
  Scenario: The firmware surface reports the running image
    When I send a GET request to "/api/v1/firmware"
    Then the response status should be 200
    And the controller should report it can update itself
    And the firmware update state should be "idle"

  @id:SYS-243
  Scenario: An image is fetched, written and selected for the next boot
    Given a firmware image server offering 16384 bytes
    When I request a firmware update from that URL
    Then the response status should be 202
    And the JSON field "type" should be "firmware_update"
    And the firmware operation should end as "succeeded"
    And the firmware update state should be "ready_to_reboot"
    And the firmware update should have written 16384 bytes

  @id:SYS-244
  Scenario: A URL the controller cannot fetch is refused before anything is erased
    When I POST JSON {"url":"file:///tmp/dali2rust.bin"} to "/api/v1/firmware/updates"
    Then the response status should be 422
    And the JSON field "error" should be "unsupported_scheme"

  @id:SYS-245
  Scenario: A server that is not there fails the operation and names the step
    Given a firmware image URL nothing is listening on
    When I request a firmware update from that URL
    Then the response status should be 202
    And the firmware operation should end as "failed"
    And the firmware update state should be "failed"
    And the firmware update error should be "fetch_failed"
