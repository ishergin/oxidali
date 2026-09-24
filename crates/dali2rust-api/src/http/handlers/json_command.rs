use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::bus_codec::confirmation_to_json_body;
use crate::confirmation_bridge::PendingConfirmationSlots;
use crate::http::dispatcher::{BusCommandDispatcher, CorrelationIdAllocator, WireDispatchParams};
use crate::http::handler::ApiHandler;
use crate::http::types::HttpResponse;
use dali2rust_bus::{BusId, BusPublisher};
use dali2rust_domain::dali::commands::dali_command_from_wire;

pub trait JsonRequestMapper: Send + Sync {
    type Request: DeserializeOwned;

    fn map(req: &Self::Request) -> Result<WireDispatchParams, HttpResponse>;
}

pub struct JsonCommandHandler<M: JsonRequestMapper> {
    dispatcher: BusCommandDispatcher,
    _phantom: PhantomData<M>,
}

impl<M: JsonRequestMapper> JsonCommandHandler<M> {
    pub fn new(
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        adapter_id: BusId,
        timeout_ms: u64,
    ) -> Self {
        Self {
            dispatcher: BusCommandDispatcher::new(
                publisher,
                slots,
                correlation,
                confirmation_to_json_body,
                "application/json",
                adapter_id,
                timeout_ms,
            ),
            _phantom: PhantomData,
        }
    }
}

impl<M: JsonRequestMapper + Send + Sync> ApiHandler for JsonCommandHandler<M> {
    fn handle_request(
        &self,
        _method: &str,
        _path: &str,
        body: &[u8],
        _params: &HashMap<String, String>,
    ) -> HttpResponse {
        if _method != "POST" {
            return HttpResponse::method_not_allowed();
        }
        let req: M::Request = match serde_json::from_slice(body) {
            Ok(r) => r,
            Err(_) => {
                return HttpResponse::json(400, br#"{"error":"invalid_json"}"#.to_vec());
            }
        };
        match M::map(&req) {
            Ok(params) => self.dispatcher.dispatch_params(&params),
            Err(resp) => resp,
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct DaliCommandRequest {
    pub wire_address: u8,
    pub command: u8,
    #[serde(default = "default_repeat")]
    pub repeat_count: u8,
}

fn default_repeat() -> u8 {
    1
}

pub fn validate_dali_wire_command(
    wire_address: u8,
    command: u8,
    repeat_count: u8,
) -> Result<(), HttpResponse> {
    dali_command_from_wire(wire_address, command, repeat_count)
        .map(|_| ())
        .map_err(|_| HttpResponse::json(400, br#"{"error":"invalid_dali_command"}"#.to_vec()))
}

pub struct DaliCommandMapper;

impl JsonRequestMapper for DaliCommandMapper {
    type Request = DaliCommandRequest;

    fn map(req: &Self::Request) -> Result<WireDispatchParams, HttpResponse> {
        validate_dali_wire_command(req.wire_address, req.command, req.repeat_count)?;
        Ok(WireDispatchParams::validated(
            req.wire_address,
            req.command,
            req.repeat_count,
        ))
    }
}

#[derive(Deserialize, Serialize)]
pub struct LevelRequest {
    pub wire_address: u8,
    pub level: u8,
}

pub struct LevelMapper;

impl JsonRequestMapper for LevelMapper {
    type Request = LevelRequest;

    fn map(req: &Self::Request) -> Result<WireDispatchParams, HttpResponse> {
        let dapc_address = req.wire_address & 0xFE;
        validate_dali_wire_command(dapc_address, req.level, 1)?;
        Ok(WireDispatchParams::validated(dapc_address, req.level, 1))
    }
}

#[derive(Deserialize, Serialize)]
pub struct RawFrameRequest {
    pub frame: u16,
    #[serde(default)]
    pub expects_backward: bool,
}

pub struct RawMapper;

impl JsonRequestMapper for RawMapper {
    type Request = RawFrameRequest;

    fn map(req: &Self::Request) -> Result<WireDispatchParams, HttpResponse> {
        let addr = ((req.frame >> 8) & 0xFF) as u8;
        let cmd = (req.frame & 0xFF) as u8;
        Ok(WireDispatchParams::raw_16(
            addr,
            cmd,
            1,
            req.expects_backward,
        ))
    }
}
