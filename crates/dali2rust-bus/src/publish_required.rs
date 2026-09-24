use log::warn;

use crate::bus::{BusChannel, BusPublisher};
use crate::frame::BusFrame;
use crate::publish::PublishResult;

pub const REQUIRED_PUBLISH_BACKOFF_MS: [u64; 10] = [0, 25, 50, 100, 150, 200, 250, 250, 250, 250];

pub const HANDLER_PUBLISH_BACKOFF_MS: [u64; 2] = [0, 15];

pub const REQUIRED_PUBLISH_UNCAPPED: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequiredPublishOutcome {
    pub queued: bool,
    pub retries: u32,
    pub slept_ms: u32,
}

const fn affords(spent_ms: u32, delay_ms: u32, budget_ms: u32) -> bool {
    delay_ms <= budget_ms.saturating_sub(spent_ms)
}

#[derive(Clone, Copy)]
pub struct RequiredPublishCounters<'a> {
    pub retried: Option<&'a core::sync::atomic::AtomicU32>,
    pub failed: Option<&'a core::sync::atomic::AtomicU32>,
}

impl<'a> RequiredPublishCounters<'a> {
    pub const NONE: Self = Self {
        retried: None,
        failed: None,
    };

    pub const fn new(
        retried: &'a core::sync::atomic::AtomicU32,
        failed: &'a core::sync::atomic::AtomicU32,
    ) -> Self {
        Self {
            retried: Some(retried),
            failed: Some(failed),
        }
    }

    fn record(self, outcome: RequiredPublishOutcome) {
        use core::sync::atomic::Ordering;
        if outcome.retries > 0 {
            if let Some(retried) = self.retried {
                retried.fetch_add(outcome.retries, Ordering::Relaxed);
            }
        }
        if !outcome.queued {
            if let Some(failed) = self.failed {
                failed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

#[inline]
pub fn publish_required_counted(
    publisher: &BusPublisher,
    channel: BusChannel,
    frame: BusFrame,
    schedule: &[u64],
    budget_ms: u32,
    label: &str,
    counters: RequiredPublishCounters<'_>,
) -> RequiredPublishOutcome {
    let outcome = publish_required(publisher, channel, frame, schedule, budget_ms, label);
    counters.record(outcome);
    outcome
}

pub fn publish_required(
    publisher: &BusPublisher,
    channel: BusChannel,
    frame: BusFrame,
    schedule: &[u64],
    budget_ms: u32,
    label: &str,
) -> RequiredPublishOutcome {
    let mut retries: u32 = 0;
    let mut slept_ms: u32 = 0;
    for (attempt, delay_ms) in schedule.iter().enumerate() {
        if *delay_ms > 0 {
            let delay_ms = u32::try_from(*delay_ms).unwrap_or(u32::MAX);
            if !affords(slept_ms, delay_ms, budget_ms) {
                break;
            }
            retries += 1;
            slept_ms += delay_ms;
            // sleep-ok: bounded ingress backoff, ADR-021
            std::thread::sleep(std::time::Duration::from_millis(u64::from(delay_ms)));
        }
        match publisher.try_publish(channel, frame.clone()) {
            PublishResult::Queued => {
                return RequiredPublishOutcome { queued: true, retries, slept_ms }
            }
            PublishResult::DroppedIngressFull if attempt + 1 < schedule.len() => {}
            other => {
                warn!("publish_required ({label}): {other:?} after {retries} retries");
                return RequiredPublishOutcome { queued: false, retries, slept_ms };
            }
        }
    }
    RequiredPublishOutcome { queued: false, retries, slept_ms }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worst_case_sleep(schedule: &[u64], budget_ms: u32) -> u32 {
        let mut spent = 0;
        for delay_ms in schedule {
            let delay_ms = *delay_ms as u32;
            if delay_ms == 0 {
                continue;
            }
            if !affords(spent, delay_ms, budget_ms) {
                break;
            }
            spent += delay_ms;
        }
        spent
    }

    #[test]
    fn a_budget_buys_a_whole_prefix_of_the_schedule() {
        let full: u32 = REQUIRED_PUBLISH_BACKOFF_MS.iter().map(|d| *d as u32).sum();
        assert_eq!(full, 1_525, "the schedule ADR-021 ships");

        assert_eq!(worst_case_sleep(&REQUIRED_PUBLISH_BACKOFF_MS, 0), 0);
        assert_eq!(worst_case_sleep(&REQUIRED_PUBLISH_BACKOFF_MS, 24), 0, "25 does not fit");
        assert_eq!(worst_case_sleep(&REQUIRED_PUBLISH_BACKOFF_MS, 25), 25);
        assert_eq!(worst_case_sleep(&REQUIRED_PUBLISH_BACKOFF_MS, 80), 75);
        assert_eq!(worst_case_sleep(&REQUIRED_PUBLISH_BACKOFF_MS, 400), 325);
        assert_eq!(worst_case_sleep(&REQUIRED_PUBLISH_BACKOFF_MS, full), full);
        assert_eq!(
            worst_case_sleep(&REQUIRED_PUBLISH_BACKOFF_MS, REQUIRED_PUBLISH_UNCAPPED),
            full,
            "uncapped must be exactly the old behaviour"
        );
    }

    #[test]
    fn an_overspent_budget_affords_nothing() {
        assert!(!affords(500, 1, 400));
        assert!(!affords(u32::MAX, 1, 0));
        assert!(affords(0, 0, 0), "a zero delay is always affordable");
    }
}
