#[cfg(test)]
pub mod shared {
    use crate::runtime::clock::StdClock;
    use crate::runtime::controller::DaliController;
    use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
    use dali2rust_domain::dali::types::DaliAddress;
    use std::sync::{Arc, Mutex};

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

    pub fn setup_controller<T: dali2rust_platform::dali::DaliTransport + Send>(
        mock: T,
    ) -> (Arc<Mutex<T>>, DaliController<T>) {
        let transport = Arc::new(Mutex::new(mock));
        let controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
        (transport, controller)
    }
}
