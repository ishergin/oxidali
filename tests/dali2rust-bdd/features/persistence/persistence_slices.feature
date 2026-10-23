@stage-F3
Feature: Persistence slices host contract

  @id:PERS-002
  Scenario: Restart preserves adapter state when the same slice store is reused
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"name":"Round Trip","enabled":false} to "/api/v1/adapters/0"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/adapters/0"
    Then the response status should be 200
    And the JSON field "name" should be "Round Trip"
    And the JSON boolean field "enabled" should be false

  @id:PERS-004
  Scenario: A schedule an operator entered survives a restart
    Given in-memory slice persistence is enabled for the host stack
    And HCL schedule "morning" exists
    When I restart the host stack
    And I send a GET request to "/api/v1/hcl-schedules/morning"
    Then the response status should be 200
    And the JSON field "algorithm" should be "stepped"
    And HCL schedule "morning" should have 1 target and 1 point

  @id:PERS-005
  Scenario: An identical attribute re-read writes nothing back to the slice store
    Given in-memory slice persistence is enabled for the host stack
    And a golden control-gear discovery script for short address 0
    When I start a discovery run for adapter 0
    Then the response status should be 202
    And the last operation eventually succeeds
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I restart the host stack
    Given a golden attribute-read identity script for short address 0
    When I start an attribute read for adapter 0 physical device 0 with memory_banks "identity"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"name":"flush-fence-a"} to "/api/v1/adapters/0"
    Then the response status should be 200
    When I PATCH JSON {"name":"flush-fence-b"} to "/api/v1/adapters/0"
    Then the response status should be 200
    Then the diagnostics persistence flush total should settle at exactly 2

  @id:PERS-006
  Scenario: A slice whose stored copy cannot be read is never written over
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"name":"Stored Name"} to "/api/v1/adapters/0"
    Then the response status should be 200
    Given reads of the stored "adapters" slice fail
    When I restart the host stack
    Then the JSON pointer "/persistence/unread_slices" at "/api/v1/diagnostics" should eventually be "1"
    When I PATCH JSON {"name":"Unsaved Name"} to "/api/v1/adapters/0"
    Then the response status should be 200
    Given reads of the stored slices succeed again
    When I restart the host stack
    And I send a GET request to "/api/v1/adapters/0"
    Then the JSON field "name" should be "Stored Name"
    And the JSON pointer "/persistence/unread_slices" at "/api/v1/diagnostics" should eventually be "0"

  @id:PERS-007
  Scenario: A slice whose stored copy could not be read at boot is read again without a restart
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"name":"Stored Name"} to "/api/v1/adapters/0"
    Then the response status should be 200
    Given reads of the stored "adapters" slice fail
    When I restart the host stack
    Then the JSON pointer "/persistence/unread_slices" at "/api/v1/diagnostics" should eventually be "1"
    When I PATCH JSON {"name":"Unsaved Name"} to "/api/v1/adapters/0"
    Then the response status should be 200
    Given reads of the stored slices succeed again
    Then the JSON pointer "/persistence/unread_slices" at "/api/v1/diagnostics" should eventually be "0"
    When I send a GET request to "/api/v1/adapters/0"
    Then the JSON field "name" should be "Stored Name"
    When I PATCH JSON {"name":"Saved Name"} to "/api/v1/adapters/0"
    Then the response status should be 200
    When I restart the host stack
    And I send a GET request to "/api/v1/adapters/0"
    Then the JSON field "name" should be "Saved Name"

  @id:PERS-008
  Scenario: A WebSocket client hears of an adapter whose stored copy is read again
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"name":"Stored Name"} to "/api/v1/adapters/0"
    Then the response status should be 200
    Given reads of the stored "adapters" slice fail
    When I restart the host stack
    Then the JSON pointer "/persistence/unread_slices" at "/api/v1/diagnostics" should eventually be "1"
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "adapters"
    Given reads of the stored slices succeed again
    Then the JSON pointer "/persistence/unread_slices" at "/api/v1/diagnostics" should eventually be "0"
    And the WebSocket client should receive a "AdapterSettingsChangedEvent" frame on channel "adapters"

  @id:PERS-009
  Scenario: A DALI settings slice read again makes the controller active, and a rule sees it
    Given in-memory slice persistence is enabled for the host stack
    When I PATCH JSON {"dt8_auto_activation_repair":false} to "/api/v1/settings/dali"
    Then the response status should be 200
    Given reads of the stored "dali_settings" slice fail
    When I restart the host stack
    Then the JSON pointer "/persistence/unread_slices" at "/api/v1/diagnostics" should eventually be "1"
    Given an open WebSocket connection
    And the WebSocket client is subscribed to "logs" at level "info"
    When I PUT JSON {"base_revision":0,"source":"rule \"back\" {\n  when controller becomes active\n  do log(\"active again\")\n}\n"} to "/api/v1/rules"
    Then the response status should be 202
    And the last operation eventually succeeds
    When I PATCH JSON {"application_active":false} to "/api/v1/settings/dali"
    Then the response status should be 200
    Given reads of the stored slices succeed again
    Then the JSON pointer "/persistence/unread_slices" at "/api/v1/diagnostics" should eventually be "0"
    And the WebSocket client should receive a "rules" log line reading "back: active again"
