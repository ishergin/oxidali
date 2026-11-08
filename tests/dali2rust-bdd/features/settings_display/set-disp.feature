@stage-R9
Feature: Settings — Display

  @id:SET-DISP-001
  Scenario: GET returns documented defaults before any PATCH
    When I send a GET request to "/api/v1/settings/display"
    Then the response status should be 200
    And the JSON boolean field "enabled" should be true
    And the JSON numeric field "i2c_address" should be 60
    And the JSON numeric field "rotation_deg" should be 0
    And the JSON numeric field "screensaver_timeout_s" should be 0
    And the JSON field "pages" should equal ["summary"]

  @id:SET-DISP-002
  Scenario: PATCH applies the requested change and returns the pages in their fixed order
    When I PATCH JSON {"i2c_address":61,"rotation_deg":180,"screensaver_timeout_s":300,"pages":["bus","summary"]} to "/api/v1/settings/display"
    Then the response status should be 200
    And the JSON numeric field "i2c_address" should be 61
    And the JSON numeric field "rotation_deg" should be 180
    And the JSON numeric field "screensaver_timeout_s" should be 300
    And the JSON field "pages" should equal ["summary","bus"]
    And the JSON boolean field "enabled" should be true

  @id:SET-DISP-003
  Scenario: PATCH round-trips through a subsequent GET
    When I PATCH JSON {"enabled":false,"pages":["network","activity"]} to "/api/v1/settings/display"
    Then the response status should be 200
    When I send a GET request to "/api/v1/settings/display"
    Then the JSON boolean field "enabled" should be false
    And the JSON field "pages" should equal ["network","activity"]
    And the JSON numeric field "i2c_address" should be 60

  @id:SET-DISP-004
  Scenario Outline: PATCH rejects a value outside its domain
    When I PATCH JSON <body> to "/api/v1/settings/display"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

    Examples:
      | body                              |
      | {"rotation_deg":90}               |
      | {"rotation_deg":270}              |
      | {"i2c_address":62}                |
      | {"screensaver_timeout_s":5}       |
      | {"screensaver_timeout_s":86401}   |
      | {"pages":[]}                      |
      | {"pages":["bus","bus"]}           |
      | {"enabled":"yes"}                 |

  @id:SET-DISP-005
  Scenario: PATCH rejects an unknown page
    When I PATCH JSON {"pages":["summary","clock"]} to "/api/v1/settings/display"
    Then the response status should be 422
    And the JSON error should be "invalid_enum"

  @id:SET-DISP-006
  Scenario: PATCH rejects an unknown field
    When I PATCH JSON {"brightness":10} to "/api/v1/settings/display"
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:SET-DISP-007
  Scenario: A refused PATCH changes nothing
    When I PATCH JSON {"rotation_deg":180,"pages":[]} to "/api/v1/settings/display"
    Then the response status should be 422
    When I send a GET request to "/api/v1/settings/display"
    Then the JSON numeric field "rotation_deg" should be 0
    And the JSON field "pages" should equal ["summary"]

  @id:SET-DISP-008
  Scenario: Display settings survive a controller restart
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"rotation_deg":180,"screensaver_timeout_s":60,"pages":["summary","network"]} to "/api/v1/settings/display"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/settings/display"
    Then the response status should be 200
    And the JSON numeric field "rotation_deg" should be 180
    And the JSON numeric field "screensaver_timeout_s" should be 60
    And the JSON field "pages" should equal ["summary","network"]

  @id:SET-DISP-009
  Scenario: The panel starts at the default address, upright and powered
    Then the display panel should eventually be configured at I2C address 60 with rotation 0
    And the display panel should eventually be powered on
    And the display panel should eventually show a row starting with "GEAR "

  @id:SET-DISP-010
  Scenario: A new address and rotation reach the panel without a restart
    When I PATCH JSON {"i2c_address":61,"rotation_deg":180} to "/api/v1/settings/display"
    Then the response status should be 200
    And the display panel should eventually be configured at I2C address 61 with rotation 180
    And the display panel should eventually be powered on

  @id:SET-DISP-011
  Scenario: Disabling the display powers the panel down, and enabling brings it back
    Then the display panel should eventually be powered on
    When I PATCH JSON {"enabled":false} to "/api/v1/settings/display"
    Then the response status should be 200
    And the display panel should eventually be powered off
    When I PATCH JSON {"enabled":true} to "/api/v1/settings/display"
    Then the response status should be 200
    And the display panel should eventually be powered on
    And the display panel should eventually show a row starting with "GEAR "

  @id:SET-DISP-012
  Scenario: The panel turns the selected pages
    When I PATCH JSON {"pages":["summary","network"]} to "/api/v1/settings/display"
    Then the response status should be 200
    And the display panel should eventually show a row starting with "NET "
    And the display panel should eventually show a row starting with "GEAR "

  @id:SET-DISP-013
  Scenario: A single selected page is the only one the panel shows
    When I PATCH JSON {"pages":["bus"]} to "/api/v1/settings/display"
    Then the response status should be 200
    And the display panel should eventually show a row starting with "FRAMES "
    And the display panel should not show a row starting with "GEAR " for 6 seconds
