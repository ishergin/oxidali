use cucumber::given;
use dali2rust_domain::dali::devices::dt8_color::COLOUR_TYPE_BYTE_MASK;
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;

use crate::steps::physical_devices::write_scripts::{
    script_level_store, POWER_ON_STORE, SYSTEM_FAILURE_STORE,
};
use crate::DaliWorld;

const WRITE_ATTEMPTS: usize = 3;
const POLICY_SHORT_ADDRESS: u8 = 0;

// POLICY-007
#[given(regex = r"^a policy write script setting both levels to (\d+) on short addresses 0 and 1$")]
async fn given_policy_write_script(world: &mut DaliWorld, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    for short in [0u8, 1] {
        script_policy_pair(&mock, short, level);
    }
}

fn script_policy_pair(mock: &MockDaliTransport, short: u8, level: u8) {
    for store in [POWER_ON_STORE, SYSTEM_FAILURE_STORE] {
        script_level_store(mock, short, store, level, (Some(COLOUR_TYPE_BYTE_MASK), Some(level)));
    }
}

// POLICY-010
#[given(regex = r"^a policy write of power-on level (\d+) that short address 0 answers with (\d+) on every attempt$")]
async fn given_policy_write_refused(world: &mut DaliWorld, level: u8, answer: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    for _ in 0..WRITE_ATTEMPTS {
        script_power_on_write(&mock, level, Some(answer));
    }
}

// POLICY-011
#[given(regex = r"^a policy write of power-on level (\d+) that short address 0 leaves unanswered$")]
async fn given_policy_write_unanswered(world: &mut DaliWorld, level: u8) {
    let mock = world.dali_mock().lock().expect("mock lock");
    mock.clear();
    script_power_on_write(&mock, level, None);
}

fn script_power_on_write(mock: &MockDaliTransport, level: u8, answer: Option<u8>) {
    script_level_store(mock, POLICY_SHORT_ADDRESS, POWER_ON_STORE, level, (Some(COLOUR_TYPE_BYTE_MASK), answer));
}
