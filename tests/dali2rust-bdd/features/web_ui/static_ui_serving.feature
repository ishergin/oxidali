@stage-X1
Feature: Embedded web UI static asset serving
  The HTTP layer serves the pre-gzipped web UI bundle alongside /api/v1:
  GET / returns the index page, /assets/* returns bundle files, unknown
  non-API paths fall back to the index (SPA deep links), and the /api/v1
  namespace keeps its JSON 404 semantics.

  @id:WEB-001
  Scenario: GET / serves the gzip-encoded index page
    When I send a GET request to "/"
    Then the response status should be 200
    And the response content type should be "text/html"
    And the response content encoding should be "gzip"
    And the response body should equal the "index.html.gz" web fixture

  @id:WEB-002
  Scenario: GET an asset path serves the gzip-encoded bundle file
    When I send a GET request to "/assets/app.js"
    Then the response status should be 200
    And the response content type should be "application/javascript"
    And the response content encoding should be "gzip"
    And the response body should equal the "app.js.gz" web fixture

  @id:WEB-003
  Scenario: GET an unknown non-API path falls back to the index page
    When I send a GET request to "/some/client/route"
    Then the response status should be 200
    And the response content type should be "text/html"
    And the response body should equal the "index.html.gz" web fixture

  @id:WEB-004
  Scenario: The SPA fallback does not shadow unknown API paths
    When I send a GET request to "/api/v1/nonexistent"
    Then the response status should be 404
    And the response content type should be "application/json"
    And the JSON error should be "not_found"
