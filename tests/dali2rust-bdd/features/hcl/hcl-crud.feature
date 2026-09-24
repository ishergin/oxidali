@stage-I3
Feature: HCL schedule CRUD

  Schedules are controller-global, so the routes are not nested under an
  adapter: one schedule drives targets on several adapters at once. A write is
  chunked onto the bus, but a client only ever sees the status and the
  read-back — the chunking is deliberately invisible here.

  @id:HCL-001
  Scenario: The list returns every stored schedule and touches no DALI bus
    Given HCL schedule "evening" exists
    And HCL schedule "morning" exists
    When I send a GET request to "/api/v1/hcl-schedules"
    Then the response status should be 200
    And the HCL schedule list should be exactly "evening, morning"

  @id:HCL-010
  Scenario: The detail read returns the full schedule contract
    Given HCL schedule "morning" exists
    When I send a GET request to "/api/v1/hcl-schedules/morning"
    Then the response status should be 200
    And the JSON field "schedule_id" should be "morning"
    And the JSON field "algorithm" should be "stepped"
    And the JSON boolean field "enabled" should be true

  @id:HCL-011
  Scenario: An unknown schedule id is not found
    When I send a GET request to "/api/v1/hcl-schedules/never-existed"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:HCL-020
  Scenario: A cross-adapter schedule stores every target and point
    When I POST JSON {"schedule_id":"morning","enabled":true,"algorithm":"stepped","active_days":["mon","tue","wed","thu","fri"],"location":null,"targets":[{"adapter_id":0,"scope":"group","group_ids":[1,5]},{"adapter_id":0,"scope":"broadcast"},{"adapter_id":1,"scope":"group","group_ids":[3]}],"points":[{"time_ref":"absolute","offset_minutes":360,"level_mode":"absolute","level":80,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":480,"level_mode":"absolute","level":200,"color_temperature_kelvin":4000}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "morning" should have 3 targets and 2 points
    And HCL schedule "morning" field "targets/1/scope" should be "broadcast"
    And HCL schedule "morning" field "targets/2/group_ids" should be [3]

  @id:HCL-027
  Scenario: A schedule posted without an id gets one from the server
    When I POST an HCL schedule without a schedule_id
    Then the response status should be 202
    And the JSON field "schedule_id" should be "schedule-1"
    And the last operation eventually succeeds

  @id:HCL-021
  Scenario: An astronomical point without a location is refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"sunset","offset_minutes":-30,"level_mode":"none","level":null,"color_temperature_kelvin":3000}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-022
  Scenario: An interpolated schedule keeps its location and astronomical points
    When I POST JSON {"schedule_id":"astro","enabled":true,"algorithm":"interpolated","active_days":["sat","sun"],"location":{"latitude_deg":55.7558,"longitude_deg":37.6173},"targets":[{"adapter_id":0,"scope":"broadcast"}],"points":[{"time_ref":"sunrise","offset_minutes":30,"level_mode":"absolute","level":100,"color_temperature_kelvin":4000},{"time_ref":"sunset","offset_minutes":-30,"level_mode":"last_active","level":null,"color_temperature_kelvin":3000}]} to "/api/v1/hcl-schedules"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "astro" should have 1 target and 2 points
    And HCL schedule "astro" field "algorithm" should be "interpolated"
    And HCL schedule "astro" field "location/latitude_deg" should be 55.7558
    And HCL schedule "astro" field "points/1/level_mode" should be "last_active"

  @id:HCL-023
  Scenario: Creating a schedule that already exists is a conflict
    Given HCL schedule "morning" exists
    When I POST an HCL schedule whose "schedule_id" is "morning"
    Then the response status should be 409
    And the JSON error should be "conflict"

  @id:HCL-024
  Scenario: A location outside the globe is refused
    When I POST an HCL schedule whose "location" is {"latitude_deg":95.0,"longitude_deg":37.6}
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-025
  Scenario: More than sixteen targets is refused
    When I POST an HCL schedule whose "targets" is [{"adapter_id":0,"scope":"broadcast"},{"adapter_id":1,"scope":"broadcast"},{"adapter_id":2,"scope":"broadcast"},{"adapter_id":3,"scope":"broadcast"},{"adapter_id":4,"scope":"broadcast"},{"adapter_id":5,"scope":"broadcast"},{"adapter_id":6,"scope":"broadcast"},{"adapter_id":7,"scope":"broadcast"},{"adapter_id":8,"scope":"broadcast"},{"adapter_id":9,"scope":"broadcast"},{"adapter_id":10,"scope":"broadcast"},{"adapter_id":11,"scope":"broadcast"},{"adapter_id":12,"scope":"broadcast"},{"adapter_id":13,"scope":"broadcast"},{"adapter_id":14,"scope":"broadcast"},{"adapter_id":15,"scope":"broadcast"},{"adapter_id":16,"scope":"broadcast"}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-026
  Scenario: The retired transition field is refused rather than ignored
    When I POST an HCL schedule whose "transition_default_ms" is 600
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:HCL-030
  Scenario: A patch replaces the fields it names and leaves the rest
    Given HCL schedule "morning" exists
    When I PATCH JSON {"enabled":false} to "/api/v1/hcl-schedules/morning"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "morning" field "enabled" should be false
    And HCL schedule "morning" should have 1 target and 1 point

  @id:HCL-028
  Scenario: A patched array replaces wholesale
    Given HCL schedule "morning" exists
    When I PATCH JSON {"points":[{"time_ref":"absolute","offset_minutes":60,"level_mode":"none","level":null,"color_temperature_kelvin":2200},{"time_ref":"absolute","offset_minutes":120,"level_mode":"none","level":null,"color_temperature_kelvin":2400},{"time_ref":"absolute","offset_minutes":180,"level_mode":"none","level":null,"color_temperature_kelvin":2600}]} to "/api/v1/hcl-schedules/morning"
    Then the response status should be 202
    And the last operation eventually succeeds
    And HCL schedule "morning" should have 1 target and 3 points
    And HCL schedule "morning" field "points/0/offset_minutes" should be 60

  @id:HCL-029
  Scenario: Patching an unknown schedule is not found
    When I PATCH JSON {"enabled":false} to "/api/v1/hcl-schedules/never-existed"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:HCL-031
  Scenario: A patch may not move a schedule to another id
    Given HCL schedule "morning" exists
    When I PATCH JSON {"schedule_id":"evening"} to "/api/v1/hcl-schedules/morning"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

  @id:HCL-032
  Scenario: A group target without groups is refused
    When I POST an HCL schedule whose "targets" is [{"adapter_id":0,"scope":"group"}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-033
  Scenario: A broadcast target carrying groups is refused
    When I POST an HCL schedule whose "targets" is [{"adapter_id":0,"scope":"broadcast","group_ids":[1,2]}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-034
  Scenario: More than twenty-four points is refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"absolute","offset_minutes":0,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":1,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":2,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":3,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":4,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":5,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":6,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":7,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":8,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":9,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":10,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":11,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":12,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":13,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":14,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":15,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":16,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":17,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":18,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":19,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":20,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":21,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":22,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":23,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":24,"level_mode":"none","level":null,"color_temperature_kelvin":2700}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-035
  Scenario: An empty target list is refused
    When I POST an HCL schedule whose "targets" is []
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-036
  Scenario: An absolute point without a level is refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"absolute","offset_minutes":600,"level_mode":"absolute","level":null,"color_temperature_kelvin":3000}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-037
  Scenario: A last-active point carrying a level is refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"absolute","offset_minutes":600,"level_mode":"last_active","level":100,"color_temperature_kelvin":3000}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-038
  Scenario: A colour-only point carrying a level is refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"absolute","offset_minutes":600,"level_mode":"none","level":50,"color_temperature_kelvin":3000}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-039
  Scenario: An unknown algorithm is an enum error, not a value error
    When I POST an HCL schedule whose "algorithm" is "exotic"
    Then the response status should be 422
    And the JSON error should be "invalid_enum"

  @id:HCL-040
  Scenario: A level above the DAPC range is refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"absolute","offset_minutes":600,"level_mode":"absolute","level":255,"color_temperature_kelvin":3000}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-041
  Scenario: A colour temperature outside the usable range is refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"absolute","offset_minutes":600,"level_mode":"none","level":null,"color_temperature_kelvin":50}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-042
  Scenario: Two points at the same effective time are refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"absolute","offset_minutes":600,"level_mode":"none","level":null,"color_temperature_kelvin":2700},{"time_ref":"absolute","offset_minutes":600,"level_mode":"none","level":null,"color_temperature_kelvin":4000}]
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-043
  Scenario: Deleting a schedule removes it
    Given HCL schedule "morning" exists
    When I send a DELETE request to "/api/v1/hcl-schedules/morning"
    Then the response status should be 204
    And HCL schedule "morning" should be gone

  @id:HCL-002
  Scenario: Deleting an unknown schedule is not found
    When I send a DELETE request to "/api/v1/hcl-schedules/never-existed"
    Then the response status should be 404
    And the JSON error should be "not_found"

  @id:HCL-044
  Scenario: A brightness key inside a point is refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"absolute","offset_minutes":600,"level_mode":"absolute","level":80,"brightness":80,"color_temperature_kelvin":3000}]
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:HCL-045
  Scenario: A mired key inside a point is refused
    When I POST an HCL schedule whose "points" is [{"time_ref":"absolute","offset_minutes":600,"level_mode":"none","level":null,"cct_mireks":333}]
    Then the response status should be 400
    And the JSON error should be "unknown_field"

  @id:HCL-046
  Scenario: A patch may not introduce a scene scope
    Given HCL schedule "morning" exists
    When I PATCH JSON {"targets":[{"adapter_id":0,"scope":"scene","group_ids":[1]}]} to "/api/v1/hcl-schedules/morning"
    Then the response status should be 422
    And the JSON error should be "unsupported_field"

  @id:HCL-047
  Scenario: A patch may not repeat a group in one target
    Given HCL schedule "morning" exists
    When I PATCH JSON {"targets":[{"adapter_id":0,"scope":"group","group_ids":[1,1]}]} to "/api/v1/hcl-schedules/morning"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-048
  Scenario: A patch may not empty the curve
    Given HCL schedule "morning" exists
    When I PATCH JSON {"points":[]} to "/api/v1/hcl-schedules/morning"
    Then the response status should be 422
    And the JSON error should be "invalid_value"

  @id:HCL-049
  Scenario: A patch may not push a point past the end of the day
    Given HCL schedule "morning" exists
    When I PATCH JSON {"points":[{"time_ref":"absolute","offset_minutes":1500,"level_mode":"none","level":null,"color_temperature_kelvin":3000}]} to "/api/v1/hcl-schedules/morning"
    Then the response status should be 422
    And the JSON error should be "invalid_value"
