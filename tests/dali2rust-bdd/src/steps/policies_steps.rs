use cucumber::given;
use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_domain::dali::pres::standard::StandardCommand;

use crate::steps::physical_devices_steps::script_addressed_config_triple;
use crate::DaliWorld;

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
