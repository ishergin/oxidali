use dali2rust_platform::mqtt::MqttSubAck;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suback {
    Pending(u32),
    Granted,
    Refused,
}

#[derive(Debug, Default)]
pub(crate) struct Subscriptions {
    sent: Vec<(String, Suback)>,
}

impl Subscriptions {
    pub(crate) fn sent(&mut self, filter: &str, message_id: u32) {
        self.sent.push((filter.to_owned(), Suback::Pending(message_id)));
    }

    pub(crate) fn holds(&self, filter: &str) -> bool {
        self.sent.iter().any(|(held, _)| held == filter)
    }

    pub(crate) fn forget(&mut self, filter: &str) {
        self.sent.retain(|(held, _)| held != filter);
    }

    pub(crate) fn settle(&mut self, ack: MqttSubAck) -> Option<&str> {
        let pending = Suback::Pending(ack.message_id);
        let (filter, state) = self.sent.iter_mut().find(|(_, state)| *state == pending)?;
        *state = if ack.granted { Suback::Granted } else { Suback::Refused };
        (!ack.granted).then_some(filter.as_str())
    }

    pub(crate) fn all_granted(&self) -> bool {
        self.sent.iter().all(|(_, state)| *state == Suback::Granted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ack(message_id: u32, granted: bool) -> MqttSubAck {
        MqttSubAck { message_id, granted }
    }

    #[test]
    fn a_suback_settles_only_the_subscription_whose_message_id_it_carries() {
        let mut subscriptions = Subscriptions::default();
        subscriptions.sent("a", 1);
        subscriptions.sent("b", 2);
        assert_eq!(subscriptions.settle(ack(2, true)), None);
        assert!(!subscriptions.all_granted(), "the first SUBSCRIBE still waits");
        assert_eq!(subscriptions.settle(ack(2, true)), None, "an id settles once");
        assert!(!subscriptions.all_granted());
        assert_eq!(subscriptions.settle(ack(1, true)), None);
        assert!(subscriptions.all_granted());
    }

    #[test]
    fn a_refusal_names_its_filter_and_holds_the_set_until_the_filter_is_forgotten() {
        let mut subscriptions = Subscriptions::default();
        subscriptions.sent("a", 1);
        subscriptions.sent("b", 2);
        assert_eq!(subscriptions.settle(ack(1, false)), Some("a"));
        assert_eq!(subscriptions.settle(ack(2, true)), None);
        assert!(!subscriptions.all_granted(), "a refused filter is not granted");
        subscriptions.forget("a");
        assert!(subscriptions.all_granted());
        assert!(!subscriptions.holds("a"));
    }

    #[test]
    fn an_acknowledgement_for_an_id_the_session_never_sent_settles_nothing() {
        let mut subscriptions = Subscriptions::default();
        subscriptions.sent("a", 1);
        assert_eq!(subscriptions.settle(ack(9, true)), None);
        assert_eq!(subscriptions.settle(ack(9, false)), None);
        assert!(!subscriptions.all_granted(), "a stray SUBACK cannot stand in for the one awaited");
        assert_eq!(subscriptions.settle(ack(1, false)), Some("a"));
    }
}
