@stage-R5
Feature: Group target state

  @id:GRP-070
  Scenario: Group target state sends a group-addressed DAPC request
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    When I PUT JSON {"power":"on","level":200} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the DALI mock transport should have sent group 7 direct arc level 200

  @id:GRP-072
  Scenario: Unsupported group color mode is rejected before DALI
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    When I PUT JSON {"color_mode":"xy","xy":{"x":4660,"y":22136}} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 422
    And the JSON error should be "unsupported_capability"
    And the DALI mock transport should have received 0 forward frame

  @id:GRP-073
  Scenario: A group nobody has classified yet is not refused its colour
    Given adapter 0 has a discovered and bound virtual lamp 1 with no confirmed colour
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    When I PUT JSON {"power":"on","level":200,"color_mode":"cct","color_temperature_kelvin":3000} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the DALI mock transport should have sent forward frame 0xA34D
    And the DALI mock transport should have sent forward frame 0xC301
    And the DALI mock transport should have sent forward frame 0x8FE7
