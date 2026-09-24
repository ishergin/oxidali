@stage-R6
Feature: Scene apply

  @id:SCN-060
  Scenario: Scene apply writes, updates and clears rows with post-program readback
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the JSON field "type" should be "scene_apply"
    And the last operation eventually succeeds
    And the scene-apply operation result should count 1 written, 0 updated and 0 cleared outcomes
    And all scripted DALI exchanges should be consumed without errors
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix applied row for virtual lamp 1 should have included true and level 100
    Given adapter 0 scene 3 desired row for virtual lamp 1 has CCT 2700K and level 179
    And adapter 0 scene 3 CCT 2700K write for short 0 level 179 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    And the scene-apply operation result should count 0 written, 1 updated and 0 cleared outcomes
    And all scripted DALI exchanges should be consumed without errors
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix applied row for virtual lamp 1 should have included true and level 179
    And the scene matrix applied row for virtual lamp 1 should echo color_mode "cct"
    When adapter 0 scene 3 desired row for virtual lamp 1 is excluded
    Given adapter 0 scene 3 clear for short 0 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    And the scene-apply operation result should count 0 written, 0 updated and 1 cleared outcomes
    And all scripted DALI exchanges should be consumed without errors
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix applied row for virtual lamp 1 should serialize excluded setpoint fields as null

  @id:SCN-061
  Scenario: Scene apply with no diff publishes no program commands
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 200
    And the JSON field "operation_id" should be absent
    And the DALI mock transport should have received 0 forward frame

  @id:SCN-062
  Scenario: Scene apply failure keeps prior applied state and scene dirty
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the last operation eventually succeeds
    When adapter 0 scene 3 desired row for virtual lamp 1 changes level to 120
    Given adapter 0 scene 3 write for short 0 level 120 fails on the DALI bus
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the last operation eventually fails
    And the scene-apply operation result should list failed virtual lamp 1
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix applied row for virtual lamp 1 should have included true and level 100
    When I send a GET request to "/api/v1/adapters/0/scenes/3"
    Then the JSON boolean field "dirty" should be true

  @id:SCN-063
  Scenario: Scene apply skips unbound virtual lamps and keeps scene dirty
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has level 100
    And adapter 0 scene 3 desired row for virtual lamp 9 has level 90
    And a DALI mock transport with no response
    And adapter 0 scene 3 write for short 0 level 100 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the JSON field "operation_id" should be non-empty
    And the last operation eventually succeeds
    And the scene-apply operation result should list skipped virtual lamp 9 with reason "vl_unbound"
    And the DALI mock transport should have received 9 forward frame
    When I send a GET request to "/api/v1/adapters/0/scenes/3"
    Then the JSON boolean field "dirty" should be true

  @id:SCN-065
  Scenario: Reject scene apply while another scene apply is active
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 1 desired row for virtual lamp 1 has level 100
    And a DALI mock transport with no response
    And the DALI transport blocks indefinitely
    When I send a POST request to "/api/v1/adapters/0/scenes/1/apply"
    Then the response status should be 202
    And the last apply operation eventually becomes active
    When I send a POST request to "/api/v1/adapters/0/scenes/1/apply"
    Then the response status should be 409
    And the JSON error should be "conflict"
    When the DALI transport unblocks
    Then the last apply operation eventually finishes
