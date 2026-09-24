@stage-R6
Feature: Scene colour readback

  @id:SCN-083
  Scenario: A programmed colour row reads its stored colour back and converges
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has CCT 2700K and level 179
    And a DALI mock transport with no response
    And adapter 0 scene 3 CCT 2700K write for short 0 level 179 is scripted
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix row for virtual lamp 1 should read back applied CCT 2700K dirty false

  @id:SCN-084
  Scenario: A gear that clamps the scene colour leaves the row visibly dirty
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has CCT 2500K and level 179
    And a DALI mock transport with no response
    And adapter 0 scene 3 CCT 2500K write for short 0 level 179 is scripted with clamped readback 370 mirek
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix row for virtual lamp 1 should read back applied CCT 2702K dirty true
    When I send a GET request to "/api/v1/adapters/0/scenes/3"
    Then the JSON boolean field "dirty" should be true

  @id:SCN-085
  Scenario: The scene-colours audit reads stored colour without recalling anything
    Given adapter 0 has a discovered and bound virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has CCT 2500K and level 90
    And a DALI mock transport with no response
    And a scene-colours audit script for short 0 with scene 3 holding 370 mirek at level 90
    When I start an attribute read for adapter 0 physical device 0 with attribute group "scene_colours" only
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix row for virtual lamp 1 should read back applied CCT 2702K dirty true

  @id:SCN-092
  Scenario: A programmed RGB row converges though the wire holds different numbers
    Given adapter 0 has a discovered and bound rgbwaf-capable virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has rgb 255,180,90 and level 179
    And a DALI mock transport with no response
    And adapter 0 scene 3 rgb write for short 0 level 179 is scripted staging 254,116,26 with readback 254,116,26
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix row for virtual lamp 1 should read back applied rgb 255,180,90 dirty false

  @id:SCN-093
  Scenario: A slot holding someone else's colour stays visibly dirty
    Given adapter 0 has a discovered and bound rgbwaf-capable virtual lamp 1 on physical device 0
    And adapter 0 scene 3 desired row for virtual lamp 1 has rgb 255,180,90 and level 179
    And a DALI mock transport with no response
    And adapter 0 scene 3 rgb write for short 0 level 179 is scripted staging 254,116,26 with readback 254,10,20
    When I send a POST request to "/api/v1/adapters/0/scenes/3/apply"
    Then the response status should be 202
    And the last operation eventually succeeds
    And all scripted DALI exchanges should be consumed without errors
    When I send a GET request to "/api/v1/adapters/0/scenes/3/matrix"
    Then the scene matrix row for virtual lamp 1 should read back applied rgb 255,55,79 dirty true
