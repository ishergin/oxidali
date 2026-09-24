@stage-I1
Feature: State-fanout integration for api and sniffer sources

  @id:SYS-210
  Scenario: Group target-state from the api updates applied member lamps
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    Given a DALI mock transport with no response
    When I PUT JSON {"power":"on","level":200} to "/api/v1/adapters/0/groups/7/target-state"
    Then the response status should be 202
    And the virtual lamp 1 runtime level should eventually be 200
    And the virtual lamp 1 last_dapc_source should be "group"

  @id:SYS-211
  Scenario: Scene recall from the api updates applied scene rows
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    Given a DALI mock transport with no response
    When I send a POST request to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 200
    And the virtual lamp 1 runtime level should eventually be 100
    And the virtual lamp 1 last_dapc_source should be "scene"

  @id:SYS-241
  Scenario: Group-scoped scene recall from the api projects the member lamp
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 desired membership includes virtual lamp 1 in group 7
    And a DALI mock transport with no response
    And adapter 0 group add for short 0 group 7 is scripted
    When I send a POST request to "/api/v1/adapters/0/groups/apply"
    Then the last operation eventually succeeds
    Given adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    Given a DALI mock transport with no response
    When I POST JSON {"scope":"group","group_id":7} to "/api/v1/adapters/0/scenes/3/recall"
    Then the response status should be 200
    And the DALI mock transport should have sent only a group 7 go-to-scene 3 frame
    And the virtual lamp 1 runtime level should eventually be 100
    And the virtual lamp 1 last_dapc_source should be "scene"

  @id:SYS-212
  Scenario: Sniffer-observed short DAPC projects runtime state without product commands
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When a foreign DAPC frame for short address 0 level 180 is observed on the bus
    Then the virtual lamp 1 runtime level should eventually be 180
    And the virtual lamp 1 last_dapc_source should be "sniffer"
    And the DALI mock transport should have received 0 forward frame

  @id:SYS-246
  Scenario: A foreign arc-power verb is resolved against the registry's own shadow
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When a foreign DAPC frame for short address 0 level 137 is observed on the bus
    Then the virtual lamp 1 runtime level should eventually be 137
    When a foreign DAPC frame for short address 0 level 0 is observed on the bus
    Then the virtual lamp 1 runtime level should eventually be 0
    When a foreign go-to-last-active-level for short address 0 is observed on the bus
    Then the virtual lamp 1 runtime level should eventually be 137
    And the DALI mock transport should have received 0 forward frame

  @id:SYS-213
  Scenario: Sniffer-observed scene recall projects applied rows
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    Given a DALI mock transport with no response
    When a foreign broadcast recall of scene 3 is observed on the bus
    Then the virtual lamp 1 runtime level should eventually be 100
    And the virtual lamp 1 last_dapc_source should be "scene"
    And the DALI mock transport should have received 0 forward frame

  @id:SYS-214
  Scenario: Unknown sniffer frames project nothing and never publish product commands
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When an unrecognized foreign frame is observed on the bus
    And a foreign DAPC frame for short address 0 level 90 is observed on the bus
    Then the virtual lamp 1 runtime level should eventually be 90
    And the DALI mock transport should have received 0 forward frame

  @id:SYS-215
  Scenario: Sniffer-observed DT8 colour writes preserve the shared ColorValue contract
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    When a foreign DT8 CCT 4000K write for short address 0 is observed on the bus
    Then the virtual lamp 1 runtime color temperature should eventually be 4000K
    And the virtual lamp 1 last_dapc_source should be null
    When a foreign DT8 xy write of 0.2 0.4 for short address 0 is observed on the bus
    Then the virtual lamp 1 runtime xy should eventually be 0.2 0.4
    When a foreign DT8 RGB write of 254 10 20 for short address 0 is observed on the bus
    Then the virtual lamp 1 runtime rgb should eventually be 255 55 79
    And the DALI mock transport should have received 0 forward frame

  @id:SYS-217
  Scenario: A sniffer-observed DAPC keeps runtime status it never observed
    Given a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the last operation eventually succeeds
    And adapter 0 physical device 0 eventually exposes the golden runtime status and memory-bank identity
    When a foreign DAPC frame for short address 0 level 180 is observed on the bus
    Then the physical device 0 state level should eventually be 180
    And adapter 0 physical device 0 eventually exposes the golden runtime status and memory-bank identity
