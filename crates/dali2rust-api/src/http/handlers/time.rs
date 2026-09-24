use std::collections::HashMap;
use std::sync::Arc;

use dali2rust_platform::wall_clock::{TimeError, WallClock};
use serde::Serialize;

use crate::http::handler::ApiHandler;
use crate::http::handlers::common::{json_err, json_stream_dto, parse_json_body};
use crate::http::handlers::resource_surface::declare_handler_shell;
use crate::http::types::HttpResponse;

#[derive(Serialize)]
struct TimeBody {
    synced: bool,
    unix_ms: Option<u64>,
    source: &'static str,
    timezone: String,
    local_minutes: Option<u16>,
    utc_offset_minutes: Option<i16>,
}

declare_handler_shell!(TimeHandler {
    clock: Arc<dyn WallClock>,
    persist_timezone: Arc<dyn Fn(&str) + Send + Sync>,
});

impl TimeHandler {
    fn snapshot(&self) -> HttpResponse {
        let local = self.clock.local();
        json_stream_dto(TimeBody {
            synced: self.clock.now_ms().is_some(),
            unix_ms: self.clock.now_ms(),
            source: self.clock.source().as_str(),
            timezone: self.clock.timezone(),
            local_minutes: local.map(|l| l.minutes_since_midnight),
            utc_offset_minutes: local.map(|l| l.utc_offset_minutes),
        })
    }

    fn apply(&self, body: &[u8]) -> Result<(), HttpResponse> {
        let value = parse_json_body(body)?;
        let object = value.as_object().ok_or_else(|| json_err(400, "invalid_json"))?;

        if let Some(tz) = object.get("timezone") {
            let tz = tz.as_str().ok_or_else(|| json_err(422, "invalid_value"))?;
            self.clock
                .set_timezone(tz)
                .map_err(|e| time_error_response(&e))?;
            (self.persist_timezone)(tz);
        }
        if let Some(unix_ms) = object.get("unix_ms") {
            let unix_ms = unix_ms.as_u64().ok_or_else(|| json_err(422, "invalid_value"))?;
            self.clock
                .set_manual_ms(unix_ms)
                .map_err(|e| time_error_response(&e))?;
        }
        Ok(())
    }
}

fn time_error_response(error: &TimeError) -> HttpResponse {
    match error {
        TimeError::NotPlausible | TimeError::InvalidTimezone => json_err(422, "invalid_value"),
    }
}

impl ApiHandler for TimeHandler {
    fn handle_request(
        &self,
        method: &str,
        _path: &str,
        body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        match method {
            "GET" => self.snapshot(),
            "PUT" => match self.apply(body) {
                Ok(()) => self.snapshot(),
                Err(response) => response,
            },
            _ => HttpResponse::method_not_allowed(),
        }
    }
}
