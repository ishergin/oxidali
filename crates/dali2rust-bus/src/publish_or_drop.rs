use log::warn;

use crate::bus::{BusChannel, BusPublisher};
use crate::frame::BusFrame;
use crate::publish::PublishResult;

pub fn publish_or_drop(
    publisher: &BusPublisher,
    channel: BusChannel,
    frame: BusFrame,
    label: &str,
) -> PublishResult {
    let result = publisher.try_publish(channel, frame);
    if result != PublishResult::Queued {
        warn!("publish_or_drop ({label}): {result:?}");
    }
    result
}
