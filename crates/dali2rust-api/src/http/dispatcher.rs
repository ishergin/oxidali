use std::sync::Arc;
use std::time::Duration;

use dali2rust_bus::{BusChannel, BusFrame, BusId, BusPublisher, PublishResult};

use crate::bus_codec::SOURCE_ID_UNSPECIFIED;
use crate::confirmation_bridge::{PendingConfirmationSlots, ReplyFormatter};
use crate::http::types::{HttpBody, HttpResponse, NO_EXTRA_HEADERS};
use dali2rust_contracts::bus::command_envelope;
use dali2rust_contracts::msg::DaliCommandPayload;

pub use dali2rust_bus::CorrelationIdAllocator;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireDispatchParams {
    pub wire_address: u8,
    pub command: u8,
    pub repeat_count: u8,
    pub raw_mode: bool,
    pub raw_expects_backward: bool,
}

impl WireDispatchParams {
    pub const fn validated(wire_address: u8, command: u8, repeat_count: u8) -> Self {
        Self {
            wire_address,
            command,
            repeat_count,
            raw_mode: false,
            raw_expects_backward: false,
        }
    }

    pub const fn raw_16(
        wire_address: u8,
        command: u8,
        repeat_count: u8,
        raw_expects_backward: bool,
    ) -> Self {
        Self {
            wire_address,
            command,
            repeat_count,
            raw_mode: true,
            raw_expects_backward,
        }
    }
}

pub struct BusCommandDispatcher {
    publisher: BusPublisher,
    slots: Arc<PendingConfirmationSlots>,
    correlation: Arc<CorrelationIdAllocator>,
    reply_formatter: ReplyFormatter,
    success_content_type: &'static str,
    adapter_id: BusId,
    timeout_ms: u64,
}

impl BusCommandDispatcher {
    pub fn new(
        publisher: BusPublisher,
        slots: Arc<PendingConfirmationSlots>,
        correlation: Arc<CorrelationIdAllocator>,
        reply_formatter: ReplyFormatter,
        success_content_type: &'static str,
        adapter_id: BusId,
        timeout_ms: u64,
    ) -> Self {
        Self {
            publisher,
            slots,
            correlation,
            reply_formatter,
            success_content_type,
            adapter_id,
            timeout_ms,
        }
    }

    pub fn dispatch(&self, wire_address: u8, command: u8, repeat_count: u8) -> HttpResponse {
        self.dispatch_params(&WireDispatchParams::validated(
            wire_address,
            command,
            repeat_count,
        ))
    }

    pub fn dispatch_params(&self, params: &WireDispatchParams) -> HttpResponse {
        let correlation_id = self.correlation.next_id();

        let wait = match self
            .slots
            .try_register(correlation_id, self.reply_formatter)
        {
            Ok(rx) => rx,
            Err(()) => {
                return HttpResponse::json(
                    503,
                    br#"{"error":"confirmation_slots_exhausted"}"#.to_vec(),
                );
            }
        };

        let frame = BusFrame::command(command_envelope(
            SOURCE_ID_UNSPECIFIED,
            correlation_id,
            self.adapter_id.0,
            None,
            DaliCommandPayload {
                wire_address: params.wire_address,
                command: params.command,
                repeat_count: params.repeat_count,
                raw_mode: params.raw_mode,
                raw_expects_backward: params.raw_expects_backward,
            },
        ));

        let pr = self.publisher.try_publish(BusChannel::Commands, frame);
        if pr != PublishResult::Queued {
            self.slots.cancel(correlation_id);
            return HttpResponse::json(503, br#"{"error":"commands_ingress_overload"}"#.to_vec());
        }

        match recv_confirmation_bytes(wait, self.timeout_ms, &self.slots, correlation_id) {
            Ok(bytes) => HttpResponse {
                status: 200,
                content_type: self.success_content_type,
                extra_headers: NO_EXTRA_HEADERS,
                body: HttpBody::Buffered(bytes),
            },
            Err(resp) => resp,
        }
    }
}

pub(crate) fn recv_confirmation_bytes(
    wait: crate::confirmation_bridge::ConfirmationHandle,
    timeout_ms: u64,
    slots: &PendingConfirmationSlots,
    correlation_id: u64,
) -> Result<Vec<u8>, HttpResponse> {
    recv_confirmation_bytes_until(
        wait,
        std::time::Instant::now() + Duration::from_millis(timeout_ms),
        slots,
        correlation_id,
    )
}

pub(crate) fn recv_confirmation_bytes_until(
    wait: crate::confirmation_bridge::ConfirmationHandle,
    deadline: std::time::Instant,
    slots: &PendingConfirmationSlots,
    correlation_id: u64,
) -> Result<Vec<u8>, HttpResponse> {
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    match wait.recv_timeout(remaining) {
        Ok(bytes) => Ok(bytes),
        Err(_) => match wait.try_recv() {
            Ok(bytes) => Ok(bytes),
            Err(_) => {
                slots.cancel(correlation_id);
                match wait.try_recv() {
                    Ok(bytes) => Ok(bytes),
                    Err(_) => {
                        slots.note_confirmation_timeout();
                        Err(HttpResponse::json(
                            504,
                            br#"{"error":"confirmation_timeout"}"#.to_vec(),
                        ))
                    }
                }
            }
        },
    }
}

pub fn publish_command_and_wait_json_confirmation(
    publisher: &BusPublisher,
    slots: &Arc<PendingConfirmationSlots>,
    correlation_id: u64,
    timeout_ms: u64,
    frame: BusFrame,
) -> Result<Vec<u8>, HttpResponse> {
    use crate::bus_codec::confirmation_to_json_body;

    let wait = match slots.try_register(correlation_id, confirmation_to_json_body) {
        Ok(rx) => rx,
        Err(()) => {
            return Err(HttpResponse::json(
                503,
                br#"{"error":"confirmation_slots_exhausted"}"#.to_vec(),
            ));
        }
    };
    let pr = publisher.try_publish(BusChannel::Commands, frame);
    if pr != PublishResult::Queued {
        slots.cancel(correlation_id);
        return Err(HttpResponse::json(
            503,
            br#"{"error":"commands_ingress_overload"}"#.to_vec(),
        ));
    }
    recv_confirmation_bytes(wait, timeout_ms, slots, correlation_id)
}

pub fn dispatch_and_wait_for_success(
    publisher: &dali2rust_bus::BusPublisher,
    slots: &std::sync::Arc<crate::confirmation_bridge::PendingConfirmationSlots>,
    correlation_id: u64,
    timeout_ms: u64,
    frame: dali2rust_bus::BusFrame,
) -> Result<(), crate::http::types::HttpResponse> {
    let conf_body = publish_command_and_wait_json_confirmation(
        publisher,
        slots,
        correlation_id,
        timeout_ms,
        frame,
    )?;
    crate::http::handlers::common::ensure_confirmation_success(&conf_body)
}

pub fn publish_batch_and_wait_for_success(
    publisher: &dali2rust_bus::BusPublisher,
    slots: &std::sync::Arc<PendingConfirmationSlots>,
    timeout_ms: u64,
    batch: Vec<(u64, dali2rust_bus::BusFrame)>,
) -> Result<(), HttpResponse> {
    use crate::bus_codec::confirmation_to_json_body;

    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    let mut waits = Vec::with_capacity(batch.len());
    for (correlation_id, _) in &batch {
        match slots.try_register(*correlation_id, confirmation_to_json_body) {
            Ok(rx) => waits.push(rx),
            Err(()) => {
                cancel_batch(slots, &batch);
                return Err(HttpResponse::json(
                    503,
                    br#"{"error":"confirmation_slots_exhausted"}"#.to_vec(),
                ));
            }
        }
    }
    let published = publish_batch_frames(publisher, &batch);
    if published < batch.len() {
        return Err(refuse_partial_batch(slots, deadline, waits, &batch, published));
    }
    collect_batch_confirmations(slots, deadline, waits, &batch)
}

fn publish_batch_frames(
    publisher: &dali2rust_bus::BusPublisher,
    batch: &[(u64, dali2rust_bus::BusFrame)],
) -> usize {
    batch
        .iter()
        .take_while(|(_, frame)| {
            dali2rust_bus::publish_required(
                publisher,
                BusChannel::Commands,
                frame.clone(),
                &dali2rust_bus::HANDLER_PUBLISH_BACKOFF_MS,
                dali2rust_bus::REQUIRED_PUBLISH_UNCAPPED,
                "http-batch",
            )
            .queued
        })
        .count()
}

fn refuse_partial_batch(
    slots: &PendingConfirmationSlots,
    deadline: std::time::Instant,
    waits: Vec<crate::confirmation_bridge::ConfirmationHandle>,
    batch: &[(u64, dali2rust_bus::BusFrame)],
    published: usize,
) -> HttpResponse {
    let mut waits = waits;
    let unpublished = waits.split_off(published);
    drop(unpublished);
    cancel_batch(slots, &batch[published..]);
    if published == 0 {
        return HttpResponse::json(503, br#"{"error":"commands_ingress_overload"}"#.to_vec());
    }
    let _ = collect_batch_confirmations(slots, deadline, waits, &batch[..published]);
    HttpResponse::json(503, br#"{"error":"partial_apply"}"#.to_vec())
}

fn collect_batch_confirmations(
    slots: &PendingConfirmationSlots,
    deadline: std::time::Instant,
    waits: Vec<crate::confirmation_bridge::ConfirmationHandle>,
    batch: &[(u64, dali2rust_bus::BusFrame)],
) -> Result<(), HttpResponse> {
    for (index, (wait, (correlation_id, _))) in waits.into_iter().zip(batch.iter()).enumerate() {
        let outcome = recv_confirmation_bytes_until(wait, deadline, slots, *correlation_id)
            .and_then(|body| crate::http::handlers::common::ensure_confirmation_success(&body));
        if let Err(resp) = outcome {
            cancel_batch(slots, &batch[index + 1..]);
            return Err(resp);
        }
    }
    Ok(())
}

fn cancel_batch(
    slots: &PendingConfirmationSlots,
    batch: &[(u64, dali2rust_bus::BusFrame)],
) {
    for (correlation_id, _) in batch {
        slots.cancel(*correlation_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::bus_codec::{confirmation_to_json_body, SOURCE_ID_UNSPECIFIED};
    use crate::confirmation_bridge::spawn_confirmation_bridge;
    use dali2rust_bus::{BusConfig, BusFrame, BusHost, BusId};
    use dali2rust_contracts::bus::{
 build_confirmation_envelope, parse_command_wire,
    };
    use dali2rust_contracts::msg::DeliveryStatus;

    fn parsed_command(frame: &BusFrame) -> dali2rust_contracts::bus::ParsedDaliCommandEnvelope {
        match frame {
            BusFrame::Command(c) => parse_command_wire(c.as_ref()).expect("dali command payload"),
            _ => panic!("expected command frame"),
        }
    }

    #[test]
    fn shared_correlation_allocator_routes_cross_endpoint_confirmations() {
        let (_host, publisher, (cmd_rx, conf_rx)) = BusHost::spawn(
            BusConfig::default(),
            |reg| {
                (
                    reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                    reg.subscribe_confirmations(4),
                )
            },
        );

        let slots = Arc::new(PendingConfirmationSlots::with_capacity(4));
        let _bridge = spawn_confirmation_bridge(conf_rx, Arc::clone(&slots));
        let correlation = Arc::new(CorrelationIdAllocator::new());
        let adapter_id = BusId(1);

        let command_dispatcher = BusCommandDispatcher::new(
            publisher.clone(),
            Arc::clone(&slots),
            Arc::clone(&correlation),
            confirmation_to_json_body,
            "application/json",
            adapter_id,
            1000,
        );
        let level_dispatcher = BusCommandDispatcher::new(
            publisher.clone(),
            Arc::clone(&slots),
            Arc::clone(&correlation),
            confirmation_to_json_body,
            "application/json",
            adapter_id,
            1000,
        );

        let command_handle = std::thread::spawn(move || command_dispatcher.dispatch(0, 0xFE, 1));
        let level_handle = std::thread::spawn(move || level_dispatcher.dispatch(0, 0x7F, 1));

        let first = cmd_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("first command");
        let second = cmd_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("second command");

        let first_env = parsed_command(&first);
        let second_env = parsed_command(&second);
        let first_command = first_env.command;
        let second_command = second_env.command;
        let (command_frame, level_frame) = if first_command == 0xFE && second_command == 0x7F {
            (first, second)
        } else if first_command == 0x7F && second_command == 0xFE {
            (second, first)
        } else {
            panic!("unexpected commands: {first_command:#04x}, {second_command:#04x}");
        };

        let level_env = parsed_command(&level_frame);
        let level_correlation = level_env.correlation_id;
        let level_confirmation = BusFrame::confirmation(build_confirmation_envelope(
            level_correlation,
            DeliveryStatus::Ok,
            0x22,
            SOURCE_ID_UNSPECIFIED,
        ));
        assert_eq!(
            publisher.try_publish(BusChannel::Confirmations, level_confirmation),
            PublishResult::Queued
        );

        let command_env = parsed_command(&command_frame);
        let command_correlation = command_env.correlation_id;
        let command_confirmation = BusFrame::confirmation(build_confirmation_envelope(
            command_correlation,
            DeliveryStatus::Ok,
            0x11,
            SOURCE_ID_UNSPECIFIED,
        ));
        assert_eq!(
            publisher.try_publish(BusChannel::Confirmations, command_confirmation),
            PublishResult::Queued
        );

        let command_response = command_handle.join().expect("command response");
        let level_response = level_handle.join().expect("level response");
        let command_json: serde_json::Value =
            serde_json::from_slice(&command_response.into_body_bytes()).expect("command json");
        let level_json: serde_json::Value =
            serde_json::from_slice(&level_response.into_body_bytes()).expect("level json");

        assert_eq!(command_json["backward_frame"].as_u64(), Some(0x11));
        assert_eq!(level_json["backward_frame"].as_u64(), Some(0x22));
    }

    #[test]
    fn publish_json_confirmation_returns_slots_exhausted_when_pool_full() {
        let slots = Arc::new(PendingConfirmationSlots::with_capacity(1));
        let _hold_first_slot = slots
            .try_register(1u64, confirmation_to_json_body)
            .expect("first slot");

        let (_host, publisher, ()) = BusHost::spawn(
            BusConfig::default(),
            |reg| {
                let _ = reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
            },
        );

        let dummy_cmd = BusFrame::command(dali2rust_contracts::bus::command_envelope(SOURCE_ID_UNSPECIFIED, 99, BusId(1).0, None, dali2rust_contracts::msg::DaliCommandPayload { wire_address: 0, command: 0xFE, repeat_count: 1, raw_mode: false, raw_expects_backward: false }));
        let err =
            publish_command_and_wait_json_confirmation(&publisher, &slots, 2u64, 1000, dummy_cmd)
                .expect_err("second register should fail");

        assert_eq!(err.status, 503);
        let body = err.into_body_bytes();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("confirmation_slots_exhausted"), "{text}");
    }

    fn batch_of<const N: usize>(publisher_bus: BusId, ids: [u64; N]) -> Vec<(u64, BusFrame)> {
        ids.iter()
            .map(|&correlation_id| {
                let frame = BusFrame::command(dali2rust_contracts::bus::command_envelope(
                    SOURCE_ID_UNSPECIFIED,
                    correlation_id,
                    publisher_bus.0,
                    None,
                    dali2rust_contracts::msg::DaliCommandPayload {
                        wire_address: 0,
                        command: 0xFE,
                        repeat_count: 1,
                        raw_mode: false,
                        raw_expects_backward: false,
                    },
                ));
                (correlation_id, frame)
            })
            .collect()
    }

    fn assert_pool_fully_free(slots: &PendingConfirmationSlots, capacity: u64) {
        let mut held = Vec::new();
        for probe in 0..capacity {
            match slots.try_register(1_000 + probe, confirmation_to_json_body) {
                Ok(handle) => held.push(handle),
                Err(()) => panic!(
                    "slot {probe} of {capacity} was still held after the batch returned; \
                     the collect loop stranded a registration"
                ),
            }
        }
    }

    #[test]
    fn batch_timeout_frees_every_slot_it_registered() {
        let (_host, publisher, (cmd_rx, _conf_rx)) = BusHost::spawn(BusConfig::default(), |reg| {
            (
                reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_confirmations(4),
            )
        });
        let slots = Arc::new(PendingConfirmationSlots::with_capacity(2));

        let err = publish_batch_and_wait_for_success(&publisher, &slots, 50, batch_of(BusId(1), [7, 8]))
            .expect_err("no confirmation arrives");
        assert_eq!(err.status, 504);

        assert!(cmd_rx.recv_timeout(Duration::from_millis(500)).is_ok());
        assert!(cmd_rx.recv_timeout(Duration::from_millis(500)).is_ok());
        assert_pool_fully_free(&slots, 2);
    }

    #[test]
    fn batch_failure_verdict_frees_every_slot_it_registered() {
        let (_host, publisher, (cmd_rx, conf_rx)) = BusHost::spawn(BusConfig::default(), |reg| {
            (
                reg.subscribe_commands(4, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES),
                reg.subscribe_confirmations(4),
            )
        });
        let slots = Arc::new(PendingConfirmationSlots::with_capacity(2));
        let _bridge = spawn_confirmation_bridge(conf_rx, Arc::clone(&slots));

        let batch = batch_of(BusId(1), [11, 12]);
        let publish_slots = Arc::clone(&slots);
        let publish_publisher = publisher.clone();
        let handle = std::thread::spawn(move || {
            publish_batch_and_wait_for_success(&publish_publisher, &publish_slots, 2_000, batch)
        });

        assert!(cmd_rx.recv_timeout(Duration::from_millis(500)).is_ok());
        assert!(cmd_rx.recv_timeout(Duration::from_millis(500)).is_ok());

        assert_eq!(
            publisher.try_publish(
                BusChannel::Confirmations,
                BusFrame::confirmation(build_confirmation_envelope(
                    11,
                    DeliveryStatus::ExecutionFailed,
                    0,
                    SOURCE_ID_UNSPECIFIED,
                )),
            ),
            PublishResult::Queued
        );

        let err = handle.join().expect("batch thread").expect_err("failed verdict");
        assert_ne!(err.status, 200);
        assert_pool_fully_free(&slots, 2);
    }

    #[test]
    fn a_partially_published_batch_is_refused_as_partial_apply() {
        let slots = Arc::new(PendingConfirmationSlots::with_capacity(3));
        let batch = batch_of(BusId(1), [31, 32, 33]);
        let waits: Vec<_> = batch
            .iter()
            .map(|(correlation_id, _)| {
                slots
                    .try_register(*correlation_id, confirmation_to_json_body)
                    .expect("pool has room")
            })
            .collect();

        let deadline = std::time::Instant::now() + Duration::from_millis(50);
        let resp = refuse_partial_batch(&slots, deadline, waits, &batch, 1);

        assert_eq!(resp.status, 503);
        let HttpBody::Buffered(bytes) = &resp.body else {
            panic!("a refusal is a buffered JSON body");
        };
        assert_eq!(
            String::from_utf8_lossy(bytes),
            r#"{"error":"partial_apply"}"#,
            "a half-applied write must not be reported as an overload — a caller \
             acts on that word, and `overload` means nothing happened"
        );
        assert_pool_fully_free(&slots, 3);
    }

    #[test]
    fn a_batch_that_published_nothing_keeps_the_overload_answer() {
        let slots = Arc::new(PendingConfirmationSlots::with_capacity(2));
        let batch = batch_of(BusId(1), [41, 42]);
        let waits: Vec<_> = batch
            .iter()
            .map(|(correlation_id, _)| {
                slots
                    .try_register(*correlation_id, confirmation_to_json_body)
                    .expect("pool has room")
            })
            .collect();

        let deadline = std::time::Instant::now() + Duration::from_millis(50);
        let resp = refuse_partial_batch(&slots, deadline, waits, &batch, 0);

        assert_eq!(resp.status, 503);
        let HttpBody::Buffered(bytes) = &resp.body else {
            panic!("a refusal is a buffered JSON body");
        };
        assert_eq!(
            String::from_utf8_lossy(bytes),
            r#"{"error":"commands_ingress_overload"}"#
        );
        assert_pool_fully_free(&slots, 2);
    }
}
