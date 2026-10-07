#[cfg(test)]
pub mod shared {
    use crate::runtime::clock::StdClock;
    use crate::runtime::controller::{DaliController, PHY_TICK_US, TX_ARM_LEAD_TICKS};
    use dali2rust_domain::dali::ses::DaliPriority;
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_domain::dali::types::DaliAddress;
    use dali2rust_platform::dali::{DaliTransport, DaliWireCounters};
    use std::sync::{Arc, Mutex};

    pub fn priorities_of(settle_us: &[u32]) -> Vec<DaliPriority> {
        settle_us
            .iter()
            .map(|requested| requested + u32::from(TX_ARM_LEAD_TICKS) * PHY_TICK_US)
            .map(|us| {
                [
                    DaliPriority::Transaction,
                    DaliPriority::UserAction,
                    DaliPriority::Configuration,
                    DaliPriority::Automatic,
                    DaliPriority::PeriodicQuery,
                ]
                .into_iter()
                .find(|p| p.contains_settle_us(us))
                .unwrap_or_else(|| panic!("{us} µs on the wire falls in no Table 22 band"))
            })
            .collect()
    }

    pub fn short_address(short: u8) -> DaliAddress {
        DaliAddress::short(short).expect("valid short address")
    }

    pub fn assert_script_consumed(transport: &Arc<Mutex<MockDaliTransport>>) {
        let guard = transport.lock().expect("mock lock");
        assert_eq!(guard.script_error(), None);
        assert_eq!(guard.scripted_exchanges_remaining(), 0);
    }

    pub fn short_raw_query_frame(address: DaliAddress, opcode: u8) -> u16 {
        let frame = dali2rust_domain::dali::frame::ForwardFrame::new(
            address.encode_address_byte() | 0x01,
            opcode,
        );
        frame.raw()
    }

    pub fn setup_controller<T: DaliTransport + Send>(
        mock: T,
    ) -> (Arc<Mutex<T>>, DaliController<T>) {
        let transport = Arc::new(Mutex::new(mock));
        let controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
        (transport, controller)
    }

    pub fn wire_counters<T: DaliTransport + Send>(
        controller: &mut DaliController<T>,
    ) -> Arc<DaliWireCounters> {
        let counters = Arc::new(DaliWireCounters::default());
        controller.set_wire_counters(Arc::clone(&counters));
        counters
    }
}
