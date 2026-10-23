use dali2rust_bus::worker_counters::bump;
use dali2rust_contracts::CORRELATION_NONE;
use dali2rust_platform::liveness::monotonic_ms;
use dali2rust_platform::mqtt::{MqttIncoming, MqttQos};

use crate::runtime::bus_out::publish_required_event;
use crate::runtime::ha_out::HaOut;
use crate::runtime::rule_topics::{self, Paced, RuleTopicCache};
use crate::runtime::worker::MqttWorkerPorts;

#[inline(never)]
pub(super) fn pace_rule_message(
    rules: &mut RuleTopicCache,
    ports: &MqttWorkerPorts,
    message: MqttIncoming,
    now_ms: u32,
) {
    if message.retained {
        return;
    }
    bump(&ports.counters.rule_messages_total);
    match rules.pacer.offer(message, now_ms) {
        Paced::Now(message) => publish_rule_message(ports, &message),
        Paced::Held { superseded: true } => bump(&ports.counters.rule_messages_coalesced_total),
        Paced::Held { superseded: false } => {}
    }
}

#[inline(never)]
pub(super) fn publish_rule_message(ports: &MqttWorkerPorts, message: &MqttIncoming) {
    publish_required_event(
        ports,
        CORRELATION_NONE,
        rule_topics::message_event(message),
        dali2rust_bus::RequiredPublishCounters {
            retried: None,
            failed: Some(&ports.counters.rule_messages_lost_total),
        },
    );
}

pub(crate) struct RulePublishBudget {
    tokens: u32,
    last_refill_ms: u32,
}

impl Default for RulePublishBudget {
    fn default() -> Self {
        Self::new(0)
    }
}

const RULE_PUBLISH_BURST: u32 = 4;
const RULE_PUBLISH_REFILL_MS: u32 = 1_000;

impl RulePublishBudget {
    pub(crate) fn new(now_ms: u32) -> Self {
        Self {
            tokens: RULE_PUBLISH_BURST,
            last_refill_ms: now_ms,
        }
    }

    fn take(&mut self, now_ms: u32) -> bool {
        let refills = now_ms.wrapping_sub(self.last_refill_ms) / RULE_PUBLISH_REFILL_MS;
        if refills > 0 {
            self.tokens = self.tokens.saturating_add(refills).min(RULE_PUBLISH_BURST);
            self.last_refill_ms = self.last_refill_ms.wrapping_add(refills * RULE_PUBLISH_REFILL_MS);
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

pub(super) fn drain_rule_publishes(
    out: &mut HaOut<'_>,
    budget: &mut RulePublishBudget,
    publishes: Vec<dali2rust_contracts::msg::MqttPublishCommand>,
) {
    for cmd in publishes {
        if !budget.take(monotonic_ms()) {
            bump(&out.ports.counters.rule_publishes_dropped_total);
            continue;
        }
        out.send(cmd.topic.as_str(), cmd.payload.as_str(), MqttQos::AtMostOnce, cmd.retain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_publish_budget_refills_when_the_monotonic_clock_wraps_to_zero() {
        let before_wrap = u32::MAX - RULE_PUBLISH_REFILL_MS / 2;
        let mut budget = RulePublishBudget::new(before_wrap);
        for _ in 0..RULE_PUBLISH_BURST {
            assert!(budget.take(before_wrap));
        }
        assert!(!budget.take(before_wrap), "the burst is spent");
        let one_interval_on = before_wrap.wrapping_add(RULE_PUBLISH_REFILL_MS);
        assert!(one_interval_on < before_wrap, "the clock stepped back through the wrap");
        assert!(budget.take(one_interval_on), "one interval refills one token");
        assert!(!budget.take(one_interval_on), "and only one");
    }
}
