@stage-X1
Feature: DALI Raw Command JSON API
  As a lighting controller
  I want to send arbitrary DALI frames via a JSON endpoint
  So that I can issue any IEC 62386-102 forward frame without domain command decoding

  @id:DALI-030
  Scenario: Send raw command with no backward expected
    Given a DALI mock transport with response 200
    When I send a JSON raw command with frame 64766 and expects_backward false
    Then the response status should be 200
    And the JSON response success should be true

  @id:DALI-031
  Scenario: Send raw query command with backward frame
    Given a DALI mock transport with response 200
    When I send a JSON raw command with frame 22944 and expects_backward true
    Then the response status should be 200
    And the JSON response success should be true

  @id:DALI-032
  Scenario: Send raw command with invalid JSON body
    Given a DALI mock transport with response 200
    When I POST invalid JSON "" to "/api/v1/dali/raw"
    Then the response status should be 400

  @id:DALI-033
  Scenario: DALI mock transport receives forward frame for raw command
    Given a DALI mock transport with no response
    When I send a JSON raw command with frame 64766 and expects_backward false
    Then the response status should be 200
    And the DALI mock transport should have received 1 forward frame
