use cucumber::given;
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::pres::standard::StandardCommand;

use crate::steps::physical_devices_steps::{
    script_addressed_config_triple, script_addressed_config_triple_answered,
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
    script_addressed_config_triple(
        mock,
        short,
        StandardCommand::SetPowerOnLevel,
        StandardCommand::QueryPowerOnLevel,
        level,
        level,
    );
    script_addressed_config_triple(
        mock,
        short,
        StandardCommand::SetSystemFailureLevel,
        StandardCommand::QuerySystemFailureLevel,
        level,
        level,
    );
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
    script_addressed_config_triple_answered(
        mock,
        POLICY_SHORT_ADDRESS,
        StandardCommand::SetPowerOnLevel,
        StandardCommand::QueryPowerOnLevel,
        level,
        answer,
    );
}
