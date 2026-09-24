use log::warn;

use crate::bus::{BusChannel, BusPublisher};
use crate::frame::BusFrame;
use crate::publish::PublishResult;

pub fn publish_or_drop(
    publisher: &BusPublisher,
    channel: BusChannel,
    frame: BusFrame,
    label: &str,
) {
    match publisher.try_publish(channel, frame) {
        PublishResult::Queued => {}
        other => warn!("publish_or_drop ({label}): {other:?}"),
    }
}
