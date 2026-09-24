use std::sync::Arc;

use dali2rust_api::coalesce::BurstCoalescer as SharedCoalescer;
use dali2rust_api::ws::coalesce_key;
use dali2rust_contracts::msg::EventEnvelope;

pub(crate) type BurstCoalescer = SharedCoalescer<Arc<EventEnvelope>>;

pub(crate) fn push_envelope(buffer: &mut BurstCoalescer, envelope: Arc<EventEnvelope>) {
    let key = coalesce_key(&envelope);
    buffer.push(key, envelope);
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::bus::event_envelope;
    use dali2rust_contracts::msg::{
        LightSetpoint, Origin, RuntimeObservation, RuntimeStateChangedEvent,
        VirtualLampChangedEvent,
    };

    fn addressed_env(
        virtual_lamp_id: Option<u8>,
        short_address: Option<u8>,
        level: u8,
    ) -> Arc<EventEnvelope> {
        Arc::new(event_envelope(
            0,
            u64::from(level),
            0,
            Some(Origin::Internal),
            RuntimeStateChangedEvent {
                adapter_id: 0,
                virtual_lamp_id,
                short_address,
                state_setpoint: LightSetpoint::from_level(level, None),
                state_observation: RuntimeObservation::default(),
                commit_source: dali2rust_contracts::msg::RuntimeSource::Api,
                commit_dimensions: LightSetpoint::from_level(level, None).dimensions(),
            },
        ))
    }

    fn runtime_env(lamp: u8, level: u8) -> Arc<EventEnvelope> {
        addressed_env(Some(lamp), Some(lamp), level)
    }

    fn lamp_changed_env(lamp: u8) -> Arc<EventEnvelope> {
        Arc::new(event_envelope(
            0,
            1,
            0,
            Some(Origin::Internal),
            VirtualLampChangedEvent {
                adapter_id: 0,
                virtual_lamp_id: lamp,
            },
        ))
    }

    fn levels(c: &mut BurstCoalescer) -> Vec<u64> {
        c.drain().map(|e| e.meta.correlation_id).collect()
    }

    #[test]
    fn last_wins_moves_the_survivor_to_its_last_write_position() {
        let mut c = BurstCoalescer::new();
        push_envelope(&mut c, runtime_env(1, 10));
        push_envelope(&mut c, lamp_changed_env(2));
        push_envelope(&mut c, runtime_env(1, 20));
        assert_eq!(c.take_superseded(), 1);
        assert_eq!(levels(&mut c), vec![1, 20]);
    }

    #[test]
    fn two_keys_aliasing_one_row_drain_in_last_write_order() {
        let mut c = BurstCoalescer::new();
        push_envelope(&mut c, addressed_env(Some(1), Some(5), 10));
        push_envelope(&mut c, addressed_env(None, Some(5), 20));
        push_envelope(&mut c, addressed_env(Some(1), Some(5), 30));
        assert_eq!(c.take_superseded(), 1);
        assert_eq!(
            levels(&mut c),
            vec![20, 30],
            "the row's last write on the wire must be the last frame out"
        );
    }

    #[test]
    fn distinct_keys_all_survive_in_order() {
        let mut c = BurstCoalescer::new();
        push_envelope(&mut c, runtime_env(1, 10));
        push_envelope(&mut c, runtime_env(2, 11));
        push_envelope(&mut c, lamp_changed_env(1));
        assert_eq!(c.take_superseded(), 0);
        assert_eq!(c.drain().count(), 3);
    }

    #[test]
    fn same_ids_different_kinds_never_merge() {
        let mut c = BurstCoalescer::new();
        push_envelope(&mut c, runtime_env(1, 10));
        push_envelope(&mut c, lamp_changed_env(1));
        assert_eq!(c.take_superseded(), 0);
        assert_eq!(c.drain().count(), 2);
    }

    #[test]
    fn superseded_count_matches_replacements_and_resets() {
        let mut c = BurstCoalescer::new();
        for level in 0..5 {
            push_envelope(&mut c, runtime_env(7, level));
        }
        assert_eq!(c.take_superseded(), 4);
        assert_eq!(c.take_superseded(), 0);
        assert_eq!(c.drain().count(), 1);
    }

}
