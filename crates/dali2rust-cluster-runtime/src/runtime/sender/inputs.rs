use core::sync::atomic::Ordering::Relaxed;

use dali2rust_contracts::msg::DaliInputEventObservedEvent;
use dali2rust_domain::registry::INPUT_INSTANCE_GROUPS;
use dali2rust_platform::liveness::monotonic_ms;

use crate::frame::{encode, Frame, Header, InputAddress, InputEvent};
use crate::inputs::OwnIdentity;

use super::super::counters::bump;
use super::super::input_announce::OwnInputReads;
use super::super::announce::Turn;
use super::super::input_line::{period_ms, InputAdmission, InputRepeat};
use super::{NodeSender, Reading};

impl NodeSender<'_> {
    pub(super) fn on_input_event(&mut self, observed: &DaliInputEventObservedEvent) {
        let reading = Reading::of(self.ports);
        if self.mode_refusal(&reading).is_some() || !reading.settings.share_input_events {
            return;
        }
        let Some(device) = observed.short_address else {
            return bump(&self.ports.counters.input_events_unaddressed_total);
        };
        let Some(event) = self.input_event(observed, device) else {
            return bump(&self.ports.counters.input_events_unsent_total);
        };
        let now_ms = monotonic_ms();
        let Some(period_ms) = period_ms(event.kind) else {
            return self.offer_input(&reading, event, now_ms);
        };
        if let Some(event) = self.outgoing.measure_input(event, period_ms, now_ms) {
            self.offer_input(&reading, event, now_ms);
        }
    }

    fn input_event(&self, observed: &DaliInputEventObservedEvent, device: u8) -> Option<InputEvent> {
        let address = InputAddress { adapter: observed.registry_adapter_id, device, instance: observed.instance_number };
        let groups = match address.instance {
            Some(instance) => self.ports.inputs.input_instance_groups(address.adapter, device, instance),
            None => instance_group_only(observed.instance_group),
        };
        let event = InputEvent {
            address,
            instance_type: observed.instance_type,
            groups,
            scheme: observed.scheme,
            kind: observed.typed,
            event_info: observed.event_info,
            value: observed.typed_value,
        };
        Some(event).filter(InputEvent::in_range)
    }

    fn offer_input(&mut self, reading: &Reading, event: InputEvent, now_ms: u32) {
        match self.outgoing.admit_input(event) {
            InputAdmission::Now(event) => self.send_input(reading, event, now_ms),
            InputAdmission::Later => {}
            InputAdmission::Dropped => bump(&self.ports.counters.input_events_unsent_total),
        }
    }

    pub(super) fn send_input(&mut self, reading: &Reading, event: InputEvent, now_ms: u32) {
        let header = self.next_header(reading);
        if self.transmit_input(&header, event) {
            bump(&self.ports.counters.input_events_sent_total);
        }
        self.outgoing.input_sent(InputRepeat { header, event }, now_ms);
    }

    pub(super) fn repeat_input(&mut self, repeat: InputRepeat) {
        self.transmit_input(&repeat.header, repeat.event);
    }

    fn transmit_input(&mut self, header: &Header, event: InputEvent) -> bool {
        let written = encode(header, &Frame::InputEvent(event), &mut self.buffer).ok();
        self.transmit(written)
    }

    pub(super) fn carry_inputs(&mut self, reading: &Reading, now_ms: u32) {
        if self.mode_refusal(reading).is_some() || !reading.settings.share_input_events {
            let abandoned = self.outgoing.abandon_inputs();
            self.ports.counters.input_events_unsent_total.fetch_add(abandoned, Relaxed);
            return;
        }
        while let Some(event) = self.outgoing.take_trailing_input(now_ms) {
            self.offer_input(reading, event, now_ms);
        }
        self.outgoing.prune_inputs(now_ms);
    }

    pub(super) fn plan_inputs(&mut self, reading: &Reading, turn: Turn) {
        let home = self.ports.home_assistant.home_assistant_settings_view();
        let identity = OwnIdentity::new(&home.controller_id, &home.discovery_prefix, home.expose_input_devices);
        let own = OwnInputReads { identity: &identity, port: &*self.ports.inputs };
        let shares = turn.running && reading.settings.share_input_events;
        self.input_announcer.plan(Turn { running: shares, ..turn }, own);
    }

    pub(super) fn send_input_page(&mut self, reading: &Reading, now_ms: u32) {
        self.page_slot.take(now_ms);
        let header = self.next_header(reading);
        let written = self.input_announcer.write_page(&mut self.buffer, &header, now_ms);
        self.transmit(written.ok());
    }
}

const fn instance_group_only(group: Option<u8>) -> [Option<u8>; INPUT_INSTANCE_GROUPS] {
    let mut groups = [None; INPUT_INSTANCE_GROUPS];
    groups[0] = group;
    groups
}
