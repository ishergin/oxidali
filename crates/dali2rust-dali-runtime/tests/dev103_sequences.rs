use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};

use dali2rust_adapters::dali::transport::mock::MockDaliTransport;
use dali2rust_adapters::dali::transport::sim::SimDaliTransport;
use dali2rust_dali_runtime::runtime::controller::DaliController;
use dali2rust_dali_runtime::runtime::executor::arbitration::probe_application_controller;
use dali2rust_dali_runtime::runtime::executor::dev103::{
    identify_device, scan_control_devices, set_event_scheme_verified, ScannedDevice,
    FRAME24_CONTENDED,
};
use dali2rust_domain::dali::dev103::{
    Device103Address, Device103Command, EventScheme, Instance103Command, InstanceAddress,
    Special103Command,
};
use dali2rust_dali_runtime::runtime::clock::StdClock;
use dali2rust_domain::dali::ses::RetryPolicy;
use dali2rust_platform::dali::{DaliWireCounters, TransferOutcome};

fn controller() -> (DaliController<MockDaliTransport>, Arc<Mutex<MockDaliTransport>>) {
    let transport = Arc::new(Mutex::new(MockDaliTransport::new()));
    let controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));
    (controller, transport)
}

fn frames24(transport: &Arc<Mutex<MockDaliTransport>>) -> Vec<[u8; 3]> {
    transport.lock().expect("mock lock").sent_frames24()
}

#[test]
fn a_scan_probes_every_address_and_never_opens_a_session() {
    let (mut controller, transport) = controller();
    let mut seen: Vec<ScannedDevice> = Vec::new();
    let summary = scan_control_devices(&mut controller, &mut |d| seen.push(d.clone()))
        .expect("scan runs on a mock");

    assert_eq!(summary.devices_found, 0);
    assert!(seen.is_empty());

    let frames = frames24(&transport);
    assert_eq!(frames.len(), 64, "one presence probe per short address");
    for (short_address, frame) in frames.iter().enumerate() {
        let expected = Device103Command::QueryNumberOfInstances
            .frame(Device103Address::Short(
                u8::try_from(short_address).expect("0..64"),
            ))
            .as_bytes();
        assert_eq!(*frame, expected, "probe {short_address}");
    }
    assert!(
        !frames
            .iter()
            .any(|f| *f == Special103Command::Initialise.frame(0xFF).as_bytes()),
        "a scan must be safe on a live bus: it opens no INITIALISE session"
    );
}

#[test]
fn a_scan_enumerates_the_instances_a_device_declares() {
    let (mut controller, transport) = controller();
    transport
        .lock()
        .expect("mock lock")
        .set_persistent_response(2);

    let mut seen: Vec<ScannedDevice> = Vec::new();
    scan_control_devices(&mut controller, &mut |d| seen.push(d.clone())).expect("scan");

    assert_eq!(seen.len(), 64, "every address answered, so every one is a device");
    let first = &seen[0];
    assert_eq!(first.short_address, 0);
    assert_eq!(first.instance_count, 2);
    assert_eq!(first.instances, vec![(0, 2), (1, 2)]);
    assert!(
        seen.iter().all(|d| !d.presence_unproven),
        "a readable answer proves presence; flagging one of these would cost \
         a real panel its Home Assistant entity (`ISSUE-104`)"
    );

    let frames = frames24(&transport);
    assert_eq!(
        frames[0],
        Device103Command::QueryNumberOfInstances
            .frame(Device103Address::Short(0))
            .as_bytes()
    );
    for instance in 0u8..2 {
        assert_eq!(
            frames[1 + usize::from(instance)],
            Instance103Command::QueryInstanceType
                .frame(
                    Device103Address::Short(0),
                    InstanceAddress::Number(instance)
                )
                .as_bytes(),
            "instance {instance} type query"
        );
    }
}

#[test]
fn setting_an_event_scheme_arms_the_dtr_sends_twice_and_reads_back() {
    let (mut controller, transport) = controller();
    transport
        .lock()
        .expect("mock lock")
        .set_persistent_response(EventScheme::DeviceInstance.code());

    set_event_scheme_verified(&mut controller, 3, 0, EventScheme::DeviceInstance)
        .expect("the device confirmed the scheme");

    let frames = frames24(&transport);
    let address = Device103Address::Short(3);
    let instance = InstanceAddress::Number(0);
    let expected = [
        Special103Command::Dtr0.frame(2).as_bytes(),
        Device103Command::QueryContentDtr0.frame(address).as_bytes(),
        Instance103Command::SetEventScheme
            .frame(address, instance)
            .as_bytes(),
        Instance103Command::SetEventScheme
            .frame(address, instance)
            .as_bytes(),
        Instance103Command::QueryEventScheme
            .frame(address, instance)
            .as_bytes(),
    ];
    assert_eq!(frames, expected);
}

#[test]
fn a_scheme_the_device_did_not_take_is_a_failure_not_a_success() {
    let (mut controller, transport) = controller();
    {
        let guard = transport.lock().expect("mock lock");
        guard.enqueue_response(2);
        guard.enqueue_response(0);
    }

    let outcome = set_event_scheme_verified(&mut controller, 3, 0, EventScheme::DeviceInstance);
    assert!(
        outcome.is_err(),
        "a scheme that reverted must not report success: every rule keyed on \
         that panel would stop matching, silently"
    );
}

#[test]
fn identify_sends_the_pair_and_nothing_else() {
    let (mut controller, transport) = controller();
    identify_device(&mut controller, 7).expect("identify");

    let expected = Device103Command::IdentifyDevice
        .frame(Device103Address::Short(7))
        .as_bytes();
    assert_eq!(
        frames24(&transport),
        vec![expected, expected],
        "IDENTIFY DEVICE is send-twice, and the 10 s window belongs to the \
         device — there is nothing to send after it"
    );
}

#[test]
fn a_collided_24_bit_frame_is_sent_again_and_the_sequence_completes() {
    for contended in [TransferOutcome::Collision, TransferOutcome::BusBusy] {
        let (mut controller, transport) = controller();
        let counters = Arc::new(DaliWireCounters::default());
        controller.set_wire_counters(Arc::clone(&counters));
        {
            let guard = transport.lock().expect("mock lock");
            guard.script_frame24_outcome(contended);
            guard.set_persistent_response(EventScheme::DeviceInstance.code());
        }

        set_event_scheme_verified(&mut controller, 3, 0, EventScheme::DeviceInstance)
            .unwrap_or_else(|e| panic!("{contended:?}: the retransmission carries the sequence, got {e:?}"));

        let frames = frames24(&transport);
        assert_eq!(
            frames[0], frames[1],
            "{contended:?}: the frame the wire refused goes out again before anything else"
        );
        assert_eq!(
            frames.len(),
            6,
            "{contended:?}: one retransmission, then the five-frame sequence as written"
        );
        assert_eq!(
            counters.exchange_retries.load(Relaxed),
            1,
            "{contended:?}: the retry is counted, so a retrying bus does not look like a clear one"
        );
    }
}

#[test]
fn a_foreign_frame_in_a_24_bit_window_is_terminal_and_not_resent() {
    let (mut controller, transport) = controller();
    transport
        .lock()
        .expect("mock lock")
        .script_frame24_outcome(TransferOutcome::ForeignInWindow);

    let outcome = set_event_scheme_verified(&mut controller, 3, 0, EventScheme::DeviceInstance);

    assert_eq!(
        outcome.err().map(|error| error.message()),
        Some(FRAME24_CONTENDED),
        "a frame the device may have executed is not a no-answer — the half of \
         the 103 frames that discard their response would read it as success"
    );
    assert_eq!(
        frames24(&transport).len(),
        1,
        "and it is not sent again: the device may have executed the copy it heard"
    );
}

#[test]
fn a_24_bit_frame_destroyed_on_every_attempt_is_still_the_contended_fault() {
    let (mut controller, transport) = controller();
    let attempts = usize::from(RetryPolicy::default().effective_max_attempts());
    {
        let guard = transport.lock().expect("mock lock");
        for _ in 0..attempts {
            guard.script_frame24_outcome(TransferOutcome::Collision);
        }
    }

    let outcome = set_event_scheme_verified(&mut controller, 3, 0, EventScheme::DeviceInstance);

    assert_eq!(
        outcome.err().map(|error| error.message()),
        Some(FRAME24_CONTENDED)
    );
    assert_eq!(
        frames24(&transport).len(),
        attempts,
        "every attempt went out, and not one more"
    );
}

#[test]
fn a_contended_read_costs_the_field_and_the_device_is_still_published() {
    let (mut controller, transport) = controller();
    {
        let guard = transport.lock().expect("mock lock");
        guard.script_frame24_outcome(TransferOutcome::Answer(0));
        guard.script_frame24_outcome(TransferOutcome::CorruptedInWindow);
    }

    let mut seen: Vec<ScannedDevice> = Vec::new();
    let summary = scan_control_devices(&mut controller, &mut |d| seen.push(d.clone()))
        .expect("a contended read must not fail the scan");

    assert_eq!(
        summary.devices_found, 1,
        "the device answered its presence probe and is published: dropping it \
         reads on every surface as a panel taken off the wall"
    );
    assert_eq!(
        summary.addresses_contended, 1,
        "and the contention is counted, so a scan that reported success is not \
         the only record of a walk that came back with holes in it"
    );
    let device = seen.first().expect("the device is published");
    assert_eq!(
        device.declarations.capabilities, None,
        "no byte is invented from a window nobody can be attributed in — the \
         field is simply unset, and the registry's merge is sticky on `None`"
    );
    let frames = frames24(&transport);
    assert!(
        frames.len() >= 64,
        "the walk must still visit every remaining address, not stop at the \
         contended one (saw {} frames)",
        frames.len()
    );
}

#[test]
fn a_collided_arbitration_probe_is_not_sent_again() {
    let (mut controller, transport) = controller();
    transport
        .lock()
        .expect("mock lock")
        .script_frame24_outcome(TransferOutcome::Collision);

    let outcome = probe_application_controller(&mut controller);

    assert_eq!(
        outcome.err().map(|error| error.message()),
        Some(FRAME24_CONTENDED),
        "a destroyed probe is reported, so the worker records no verdict"
    );
    assert_eq!(
        frames24(&transport).len(),
        1,
        "and the next probe is one interval away, not one break away"
    );
}

use dali2rust_domain::dali::controller::DaliApplicationController;
use dali2rust_domain::dali::ses::{DaliPriority, TransactionPriority};

fn priorities24(transport: &Arc<Mutex<MockDaliTransport>>) -> Vec<DaliPriority> {
    let lead = 3 * 104;
    transport
        .lock()
        .expect("mock lock")
        .sent_frame24_settle_us()
        .iter()
        .map(|requested| requested + lead)
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

#[test]
fn the_arbitration_probe_leaves_at_priority_five() {
    let (mut controller, transport) = controller();

    controller
        .with_wire_class(Some(TransactionPriority::PeriodicQuery), |c| {
            probe_application_controller(c)
        })
        .expect("a silent mock is an unowned bus, not a fault");

    assert_eq!(priorities24(&transport), vec![DaliPriority::PeriodicQuery]);
}

#[test]
fn a_dtr_armed_103_write_continues_at_priority_one_after_its_proof() {
    let (mut controller, transport) = controller();
    transport
        .lock()
        .expect("mock lock")
        .set_persistent_response(EventScheme::DeviceInstance.code());

    controller
        .with_wire_class(Some(TransactionPriority::Configuration), |c| {
            set_event_scheme_verified(c, 3, 0, EventScheme::DeviceInstance)
        })
        .expect("the device confirmed the scheme");

    assert_eq!(
        priorities24(&transport),
        vec![
            DaliPriority::Configuration,
            DaliPriority::Transaction,
            DaliPriority::Transaction,
            DaliPriority::Transaction,
            DaliPriority::Configuration,
        ],
        "DTR0 at the class, proof + pair at priority 1, read-back a first frame"
    );
}

#[test]
fn a_broadcast_read_back_is_contended_the_moment_a_second_panel_answers() {
    let mut sim = SimDaliTransport::demo_bus();
    let fleet = sim.input_fleet_mut();
    let staged = 0x5A;
    fleet.exchange24(Special103Command::Dtr0.frame(staged).as_bytes(), false);

    let broadcast = fleet.exchange24(
        Device103Command::QueryContentDtr0
            .frame(Device103Address::Broadcast)
            .as_bytes(),
        true,
    );
    assert_eq!(
        broadcast,
        TransferOutcome::CorruptedInWindow,
        "two devices answering one backward window is a violating frame \
         (§8.2.5) — an answer with no readable content, and it stays one when \
         both of them hold the same byte"
    );

    for short in [0u8, 1] {
        let addressed = fleet.exchange24(
            Device103Command::QueryContentDtr0
                .frame(Device103Address::Short(short))
                .as_bytes(),
            true,
        );
        assert_eq!(
            addressed,
            TransferOutcome::Answer(staged),
            "one answerer, one readable byte — panel {short}"
        );
    }
}

#[test]
fn an_instance_write_lands_on_a_segment_with_two_panels() {
    let transport = Arc::new(Mutex::new(SimDaliTransport::demo_bus()));
    let mut controller = DaliController::new(Arc::clone(&transport), Box::new(StdClock::new()));

    for short in [0u8, 1] {
        set_event_scheme_verified(&mut controller, short, 0, EventScheme::Device).unwrap_or_else(
            |e| {
                panic!(
                    "panel {short}: the instance write did not complete on a two-panel \
                     segment ({e:?}) — `stage_dtr0`'s proof must be ADDRESSED"
                )
            },
        );
    }
}

#[test]
fn a_transient_violation_is_asked_again_and_does_not_manufacture_a_device() {
    let (mut controller, transport) = controller();
    {
        let guard = transport.lock().expect("mock lock");
        guard.script_frame24_outcome(TransferOutcome::CorruptedInWindow);
        guard.script_frame24_outcome(TransferOutcome::NoAnswer);
    }

    let mut seen: Vec<ScannedDevice> = Vec::new();
    let summary = scan_control_devices(&mut controller, &mut |d| seen.push(d.clone()))
        .expect("scan");

    assert_eq!(
        summary.devices_found, 0,
        "a transient violation must not publish a device: it did, and the \
         phantoms reached the registry with `ha_expose: true`"
    );
    assert!(seen.is_empty(), "nothing may be published for an empty address");
    assert_eq!(
        summary.addresses_reprobed, 1,
        "and the re-probe must be COUNTED — a run that leaves this at zero has \
         not exercised the mechanism, whatever else it proves"
    );
}

#[test]
fn a_violation_that_repeats_is_still_a_device() {
    let (mut controller, transport) = controller();
    {
        let guard = transport.lock().expect("mock lock");
        guard.script_frame24_outcome(TransferOutcome::CorruptedInWindow);
        guard.script_frame24_outcome(TransferOutcome::CorruptedInWindow);
    }

    let mut seen: Vec<ScannedDevice> = Vec::new();
    let summary = scan_control_devices(&mut controller, &mut |d| seen.push(d.clone()))
        .expect("scan");

    assert_eq!(
        summary.devices_found, 1,
        "a violation that repeats is something that is really there, and \
         dropping it would declare a duplicated address empty"
    );
    assert_eq!(summary.addresses_reprobed, 1, "asked twice, once");
    let device = seen.first().expect("published");
    assert_eq!(
        device.instance_count, 0,
        "the count it could not state stays unstated rather than invented"
    );
    assert!(
        device.presence_unproven,
        "nothing readable ever came back, and the registry needs that fact to \
         withhold the Home Assistant entity — publishing the address while \
         silently calling its presence proven is `ISSUE-104` one storey up"
    );
}
