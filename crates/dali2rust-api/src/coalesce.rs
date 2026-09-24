use crate::ws::CoalesceKey;

const RETAINED_CAPACITY: usize = 128;

pub struct BurstCoalescer<T> {
    entries: Vec<(Option<CoalesceKey>, T)>,
    superseded: u32,
}

impl<T> BurstCoalescer<T> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            superseded: 0,
        }
    }

    pub fn push(&mut self, key: Option<CoalesceKey>, item: T) {
        if key.is_some() {
            if let Some(index) = self.entries.iter().position(|(k, _)| *k == key) {
                self.entries.remove(index);
                self.superseded = self.superseded.saturating_add(1);
            }
        }
        self.entries.push((key, item));
    }

    pub fn take_superseded(&mut self) -> u32 {
        std::mem::take(&mut self.superseded)
    }

    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.entries.drain(..).map(|(_, item)| item)
    }

    pub fn pop_front(&mut self) -> Option<T> {
        if self.entries.is_empty() {
            return None;
        }
        Some(self.entries.remove(0).1)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn release_burst(&mut self) {
        if self.entries.capacity() > RETAINED_CAPACITY {
            self.entries.shrink_to(RETAINED_CAPACITY);
        }
    }
}


impl<T> Default for BurstCoalescer<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{BurstCoalescer, RETAINED_CAPACITY};

    fn key(kind: u16) -> Option<crate::ws::CoalesceKey> {
        crate::ws::coalesce_key(&test_envelope(kind))
    }

    fn test_envelope(virtual_lamp_id: u16) -> dali2rust_contracts::msg::EventEnvelope {
        use dali2rust_contracts::msg::{
            LightSetpoint, Origin, PowerState, RuntimeObservation, RuntimeStateChangedEvent,
        };
        dali2rust_contracts::bus::event_envelope(
            dali2rust_contracts::SOURCE_ID_UNSPECIFIED,
            0,
            0,
            Some(Origin::Internal),
            RuntimeStateChangedEvent {
                adapter_id: 0,
                virtual_lamp_id: Some(virtual_lamp_id as u8),
                short_address: None,
                state_setpoint: LightSetpoint {
                    power: PowerState::On,
                    ..LightSetpoint::default()
                },
                state_observation: RuntimeObservation::default(),
                commit_source: dali2rust_contracts::msg::RuntimeSource::Api,
                commit_dimensions: LightSetpoint {
                    power: PowerState::On,
                    ..LightSetpoint::default()
                }
                .dimensions(),
            },
        )
    }

    #[test]
    fn an_oversized_burst_is_released_back_to_the_retained_capacity() {
        let mut c: BurstCoalescer<u32> = BurstCoalescer::new();
        for lamp in 0..256u32 {
            c.push(key(lamp as u16), lamp);
        }
        assert!(
            c.entries.capacity() > RETAINED_CAPACITY,
            "precondition: the burst must outgrow what a drain retains"
        );
        assert_eq!(c.drain().count(), 256);
        c.release_burst();
        assert!(c.entries.capacity() <= RETAINED_CAPACITY);
    }

    #[test]
    fn a_later_write_supersedes_one_still_queued_behind_the_current_item() {
        let mut c: BurstCoalescer<&str> = BurstCoalescer::new();
        c.push(key(1), "first");
        c.push(key(2), "other");
        assert_eq!(c.pop_front(), Some("first"));

        c.push(key(2), "newer");
        assert_eq!(c.pop_front(), Some("newer"), "the queued one was replaced");
        assert!(c.is_empty());
        assert_eq!(c.take_superseded(), 1);
    }

    #[test]
    fn keyless_items_pass_through_unmerged() {
        let mut c: BurstCoalescer<u8> = BurstCoalescer::new();
        c.push(None, 1);
        c.push(None, 2);
        assert_eq!(c.drain().collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(c.take_superseded(), 0);
    }
}
