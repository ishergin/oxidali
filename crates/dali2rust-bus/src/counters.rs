#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelCounters {
    pub publish_attempted: u32,
    pub publish_queued: u32,
    pub ingress_overflow: u32,
    pub oversize_rejected: u32,
    pub kind_mismatch: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubscriberCounters {
    pub delivered: u32,
    pub receiver_overflow: u32,
    pub name: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BusCounters {
    pub commands: ChannelCounters,
    pub confirmations: ChannelCounters,
    pub events: ChannelCounters,
    pub commands_unrouted: u32,
    pub delivery_rejected_dropped: u32,
    pub command_subscribers: Vec<SubscriberCounters>,
    pub confirmation_subscribers: Vec<SubscriberCounters>,
    pub event_subscribers: Vec<SubscriberCounters>,
}
