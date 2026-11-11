use std::sync::Arc;

use dali2rust_bus::{BusId, CommandArrivalObserver};
use dali2rust_contracts::msg::{BusCommandPayload, BusEnvelope, Origin};
use dali2rust_domain::dali::ses::TransactionPriority;
use dali2rust_platform::dali::{WireActivity, WirePriority, YieldGranularity};

const READ_ATTRIBUTES: &str = "DaliReadAttributesCommand";
const SET_TARGET_STATE: &str = "DaliSetTargetStateCommand";
const RECALL_LAST_ACTIVE_LEVEL: &str = "DaliRecallLastActiveLevelCommand";
const RECALL_SCENE: &str = "DaliRecallSceneCommand";
const STOP_FADE: &str = "DaliStopFadeCommand";
const RECALL_LEVEL_PRESET: &str = "DaliRecallLevelPresetCommand";
const DIM: &str = "DaliDimCommand";

struct WireClass {
    variant: &'static str,
    priority: WirePriority,
    granularity: YieldGranularity,
    drives_lamp: bool,
    class: TransactionPriority,
}

const fn class(
    variant: &'static str,
    priority: WirePriority,
    granularity: YieldGranularity,
    drives_lamp: bool,
    class: TransactionPriority,
) -> WireClass {
    WireClass {
        variant,
        priority,
        granularity,
        drives_lamp,
        class,
    }
}

const WIRE_CLASSES: &[WireClass] = &[
    // IEC 62386-103 §9.13.1
    class(
        "DaliCommandPayload",
        WirePriority::Interactive,
        YieldGranularity::Never,
        true,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        SET_TARGET_STATE,
        WirePriority::Interactive,
        YieldGranularity::Never,
        true,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        RECALL_SCENE,
        WirePriority::Interactive,
        YieldGranularity::Never,
        true,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        STOP_FADE,
        WirePriority::Interactive,
        YieldGranularity::Never,
        true,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        RECALL_LEVEL_PRESET,
        WirePriority::Interactive,
        YieldGranularity::Never,
        true,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-101 §9.4
    class(
        DIM,
        WirePriority::Interactive,
        YieldGranularity::Never,
        true,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        RECALL_LAST_ACTIVE_LEVEL,
        WirePriority::Interactive,
        YieldGranularity::Never,
        true,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "DaliCommissioningStepCommand",
        WirePriority::Interactive,
        YieldGranularity::Never,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        READ_ATTRIBUTES,
        WirePriority::Attended,
        YieldGranularity::Frame,
        false,
        TransactionPriority::PeriodicQuery,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "DaliReadMemoryBankCommand",
        WirePriority::Attended,
        YieldGranularity::Frame,
        false,
        TransactionPriority::PeriodicQuery,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "DaliWriteAttributesCommand",
        WirePriority::Attended,
        YieldGranularity::Step,
        false,
        TransactionPriority::Configuration,
    ),
    // IEC 62386-102 §9.10.2
    class(
        "DaliWriteMemoryLuminaireCommand",
        WirePriority::Attended,
        YieldGranularity::Step,
        false,
        TransactionPriority::Configuration,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "DaliDiscoverDevicesCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "DaliProgramGroupMembershipCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::Configuration,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "DaliProgramSceneCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::Configuration,
    ),
    // IEC 62386-103 §9.13.1
    // IEC 62386-102 §9.14.3.1
    class(
        "DaliIdentifyDeviceCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "DaliBusHealthProbeCommand",
        WirePriority::Unattended,
        YieldGranularity::Frame,
        false,
        TransactionPriority::PeriodicQuery,
    ),
    // IEC 62386-103 §9.13.1
    // IEC 62386-101 §9.2, §9.3
    class(
        "Dali103HandoverCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::UserAction,
    ),
    // DiiA 351 §7
    class(
        "Dali103ArbitrationProbeCommand",
        WirePriority::Unattended,
        YieldGranularity::Never,
        false,
        TransactionPriority::PeriodicQuery,
    ),
    // IEC 62386-103 §11.10.21
    class(
        "Dali103TestFrameCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.12.2
    class(
        "Dali103PowerNotificationCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "DaliAddressingCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "DaliReplaceDeviceCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "Dali103ScanCommand",
        WirePriority::Attended,
        YieldGranularity::Frame,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "Dali103CommissionCommand",
        WirePriority::Attended,
        YieldGranularity::Never,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "Dali103InstanceConfigureCommand",
        WirePriority::Attended,
        YieldGranularity::Step,
        false,
        TransactionPriority::Configuration,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "Dali103IdentifyCommand",
        WirePriority::Attended,
        YieldGranularity::Frame,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "Dali103FeedbackConfigureCommand",
        WirePriority::Attended,
        YieldGranularity::Step,
        false,
        TransactionPriority::Configuration,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "Dali103FeedbackDriveCommand",
        WirePriority::Interactive,
        YieldGranularity::Frame,
        false,
        TransactionPriority::UserAction,
    ),
    // IEC 62386-103 §9.13.1
    class(
        "Dali103InstanceActionCommand",
        WirePriority::Interactive,
        YieldGranularity::Frame,
        false,
        TransactionPriority::UserAction,
    ),
];

fn row(variant: &str) -> Option<&'static WireClass> {
    WIRE_CLASSES.iter().find(|row| row.variant == variant)
}

fn refine_by_origin(variant: &str, default: WirePriority, origin: Origin) -> WirePriority {
    match (variant, origin) {
        (READ_ATTRIBUTES, Origin::Poller) => WirePriority::Unattended,
        (SET_TARGET_STATE | RECALL_LAST_ACTIVE_LEVEL, Origin::Hcl) => WirePriority::Attended,
        (SET_TARGET_STATE | RECALL_LAST_ACTIVE_LEVEL | RECALL_SCENE | RECALL_LEVEL_PRESET, Origin::Mqtt) => {
            WirePriority::Attended
        }
        (SET_TARGET_STATE | RECALL_LAST_ACTIVE_LEVEL | RECALL_SCENE | RECALL_LEVEL_PRESET, Origin::Rules) => {
            WirePriority::Interactive
        }
        _ => default,
    }
}

fn refine_class_by_origin(
    variant: &str,
    default: TransactionPriority,
    origin: Origin,
) -> TransactionPriority {
    match (variant, origin) {
        // IEC 62386-103 §9.13.1
        (SET_TARGET_STATE | RECALL_LAST_ACTIVE_LEVEL, Origin::Hcl) => {
            TransactionPriority::Automatic
        }
        _ => default,
    }
}

pub fn wire_priority(variant: &str, origin: Origin) -> Option<WirePriority> {
    row(variant).map(|row| refine_by_origin(variant, row.priority, origin))
}

pub fn transaction_priority(variant: &str, origin: Origin) -> Option<TransactionPriority> {
    row(variant).map(|row| refine_class_by_origin(variant, row.class, origin))
}

pub fn drives_lamp_state(variant: &str) -> Option<bool> {
    row(variant).map(|row| row.drives_lamp)
}

pub fn yield_granularity(variant: &str) -> Option<YieldGranularity> {
    row(variant).map(|row| row.granularity)
}

pub(crate) fn command_wire_priority(
    meta: &BusEnvelope,
    payload: &BusCommandPayload,
    adapter_id: BusId,
) -> Option<WirePriority> {
    if BusId(meta.target_adapter_id) != adapter_id {
        return None;
    }
    wire_priority(payload.variant_name(), meta.origin)
}

pub(crate) fn command_transaction_priority(
    meta: &BusEnvelope,
    payload: &BusCommandPayload,
    adapter_id: BusId,
) -> Option<TransactionPriority> {
    if BusId(meta.target_adapter_id) != adapter_id {
        return None;
    }
    transaction_priority(payload.variant_name(), meta.origin)
}

pub struct WireArrivalObserver {
    gate: Arc<WireActivity>,
    adapter_id: BusId,
}

impl WireArrivalObserver {
    pub fn new(gate: Arc<WireActivity>, adapter_id: BusId) -> Self {
        Self { gate, adapter_id }
    }
}

pub const OPERATOR_BACKGROUND_QUEUE_DEPTH: usize =
    dali2rust_domain::registry::MAX_SHORT_ADDRESSES;

const QUEUE_PLACES: u32 = OPERATOR_BACKGROUND_QUEUE_DEPTH as u32;

fn waits_behind_the_queue(priority: WirePriority, origin: Origin) -> bool {
    origin == Origin::Api && priority != WirePriority::Interactive
}

pub(crate) fn holds_a_queue_place(
    meta: &BusEnvelope,
    payload: &BusCommandPayload,
    adapter_id: BusId,
) -> bool {
    command_wire_priority(meta, payload, adapter_id)
        .is_some_and(|priority| waits_behind_the_queue(priority, meta.origin))
}

impl CommandArrivalObserver for WireArrivalObserver {
    fn admits_command(&self, meta: &BusEnvelope, payload: &BusCommandPayload) -> bool {
        !holds_a_queue_place(meta, payload, self.adapter_id)
            || self.gate.reserve_queue_place(QUEUE_PLACES)
    }

    fn observe_command(&self, meta: &BusEnvelope, payload: &BusCommandPayload) {
        if let Some(priority) = command_wire_priority(meta, payload, self.adapter_id) {
            self.gate.note_arrival(priority);
        }
    }

    fn withdraw_command(&self, meta: &BusEnvelope, payload: &BusCommandPayload) {
        if holds_a_queue_place(meta, payload, self.adapter_id) {
            self.gate.release_queue_place();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dali2rust_contracts::msg::{
        DaliCommandPayload, RegistryRuntimeUpdateCommand, RuntimeRegistryUpdateEntry,
    };

    const OURS: BusId = BusId(0);

    fn meta(origin: Origin, target_adapter_id: u16) -> BusEnvelope {
        BusEnvelope {
            target_adapter_id,
            origin,
            ..Default::default()
        }
    }

    fn dali_command() -> BusCommandPayload {
        DaliCommandPayload {
            wire_address: 2,
            command: 5,
            repeat_count: 1,
            raw_mode: false,
            raw_expects_backward: false,
        }
        .into()
    }

    fn projector_runtime_update() -> BusCommandPayload {
        RegistryRuntimeUpdateCommand::internal(
            0,
            RuntimeRegistryUpdateEntry::sniffer_level(1, 128, 0),
        )
        .into()
    }

    #[test]
    fn wire_priority_table_covers_every_handled_command() {
        for variant in crate::runtime::dali_worker::DALI_WORKER_HANDLED_COMMANDS {
            assert!(
                wire_priority(variant, Origin::Api).is_some(),
                "{variant} reaches the DALI worker with no wire priority"
            );
            assert!(
                yield_granularity(variant).is_some(),
                "{variant} reaches the DALI worker with no yield granularity"
            );
            assert!(
                drives_lamp_state(variant).is_some(),
                "{variant} reaches the DALI worker unclassified for lamp-state FIFO"
            );
            assert!(
                transaction_priority(variant, Origin::Api).is_some(),
                "{variant} reaches the DALI worker with no IEC 62386-103 wire class"
            );
        }
    }

    #[test]
    fn exactly_the_lamp_driving_kinds_are_flagged_for_fifo() {
        let mut flagged: Vec<&str> = crate::runtime::dali_worker::DALI_WORKER_HANDLED_COMMANDS
            .iter()
            .copied()
            .filter(|variant| drives_lamp_state(variant) == Some(true))
            .collect();
        flagged.sort_unstable();
        let mut expected = vec![
            "DaliCommandPayload",
            SET_TARGET_STATE,
            RECALL_SCENE,
            RECALL_LAST_ACTIVE_LEVEL,
            STOP_FADE,
            RECALL_LEVEL_PRESET,
            DIM,
        ];
        expected.sort_unstable();
        assert_eq!(flagged, expected, "the set of lamp-driving commands moved");
    }

    #[test]
    fn an_ha_command_is_attended_so_a_slider_wins_and_two_ha_commands_do_not_preempt_each_other() {
        for variant in [SET_TARGET_STATE, RECALL_LAST_ACTIVE_LEVEL, RECALL_SCENE, RECALL_LEVEL_PRESET] {
            assert_eq!(
                wire_priority(variant, Origin::Mqtt),
                Some(WirePriority::Attended),
                "{variant} from Home Assistant must yield to an operator at the keyboard"
            );
            assert_eq!(
                wire_priority(variant, Origin::Api),
                Some(WirePriority::Interactive),
                "{variant} from the web UI holds the httpd task and must outrank it"
            );
        }
    }

    #[test]
    fn a_held_button_s_dim_outranks_the_poller_s_read_back_and_never_yields() {
        assert_eq!(wire_priority(DIM, Origin::Rules), Some(WirePriority::Interactive));
        assert_eq!(transaction_priority(DIM, Origin::Rules), Some(TransactionPriority::UserAction));
        assert_eq!(yield_granularity(DIM), Some(YieldGranularity::Never));
    }

    #[test]
    fn a_command_from_home_assistant_is_not_background_work() {
        assert!(!Origin::Mqtt.is_background());
        assert!(Origin::Poller.is_background());
    }

    #[test]
    fn no_command_kind_opens_a_transaction_at_priority_one() {
        let origins = [
            Origin::Api,
            Origin::Mqtt,
            Origin::Hcl,
            Origin::Cluster,
            Origin::AdapterProxy,
            Origin::Sniffer,
            Origin::Poller,
            Origin::Registry,
            Origin::Internal,
            Origin::Rules,
        ];
        for row in WIRE_CLASSES {
            for origin in origins {
                assert_ne!(
                    transaction_priority(row.variant, origin),
                    Some(TransactionPriority::Transaction),
                    "{} from {origin:?} would open at the priority a transaction continues with",
                    row.variant
                );
            }
        }
    }

    #[test]
    fn the_table_classifies_nothing_the_worker_does_not_execute() {
        for row in WIRE_CLASSES {
            assert!(
                crate::runtime::dali_worker::DALI_WORKER_HANDLED_COMMANDS.contains(&row.variant),
                "{} is classified but never dispatched",
                row.variant
            );
        }
    }

    #[test]
    fn an_operator_command_for_this_adapter_is_interactive() {
        assert_eq!(
            command_wire_priority(&meta(Origin::Api, 0), &dali_command(), OURS),
            Some(WirePriority::Interactive)
        );
    }

    #[test]
    fn a_ui_attribute_read_is_attended_and_the_pollers_is_not() {
        assert_eq!(
            wire_priority(READ_ATTRIBUTES, Origin::Api),
            Some(WirePriority::Attended)
        );
        assert_eq!(
            wire_priority(READ_ATTRIBUTES, Origin::Poller),
            Some(WirePriority::Unattended)
        );
    }

    #[test]
    fn an_hcl_setpoint_is_attended_while_an_operators_is_interactive() {
        assert_eq!(
            wire_priority(SET_TARGET_STATE, Origin::Hcl),
            Some(WirePriority::Attended)
        );
        assert_eq!(
            wire_priority(SET_TARGET_STATE, Origin::Api),
            Some(WirePriority::Interactive)
        );
    }

    #[test]
    fn another_adapters_operator_command_is_not_ours_to_yield_to() {
        assert_eq!(
            command_wire_priority(&meta(Origin::Api, 1), &dali_command(), OURS),
            None
        );
    }

    #[test]
    fn a_registry_update_never_touches_the_wire_and_so_has_no_priority() {
        assert_eq!(
            command_wire_priority(&meta(Origin::Internal, 0), &projector_runtime_update(), OURS),
            None
        );
    }

    fn operator_read() -> BusCommandPayload {
        dali2rust_contracts::msg::DaliReadAttributesCommand {
            registry_adapter_id: 0,
            short_address: 5,
            attribute_groups_mask: dali2rust_contracts::msg::DaliAttributeGroup::RuntimeStatus
                .mask_bit(),
            memory_banks: dali2rust_contracts::msg::MemoryBankReadPreset::None,
        }
        .into()
    }

    fn observer_at_depth(depth: usize) -> WireArrivalObserver {
        let gate = Arc::new(WireActivity::new());
        gate.note_queue_depth(depth);
        WireArrivalObserver::new(gate, OURS)
    }

    const FULL: usize = OPERATOR_BACKGROUND_QUEUE_DEPTH;

    #[test]
    fn an_operators_background_command_is_refused_once_the_queue_holds_its_depth() {
        let read = operator_read();
        assert!(observer_at_depth(FULL - 1).admits_command(&meta(Origin::Api, 0), &read));
        assert!(
            !observer_at_depth(FULL).admits_command(&meta(Origin::Api, 0), &read),
            "a full queue still took an operator's read"
        );
    }

    #[test]
    fn an_operators_lamp_command_is_admitted_however_deep_the_queue() {
        let full = observer_at_depth(usize::MAX);
        assert!(full.admits_command(&meta(Origin::Api, 0), &dali_command()));
    }

    #[test]
    fn paced_and_internal_work_is_admitted_however_deep_the_queue() {
        let full = observer_at_depth(usize::MAX);
        for origin in [Origin::Poller, Origin::Internal, Origin::Mqtt, Origin::Hcl] {
            assert!(
                full.admits_command(&meta(origin, 0), &operator_read()),
                "{origin:?} paces its own work and was refused"
            );
        }
    }

    #[test]
    fn an_admitted_operator_command_holds_its_place_until_it_is_withdrawn() {
        let observer = observer_at_depth(FULL - 1);
        let api = meta(Origin::Api, 0);
        assert!(observer.admits_command(&api, &operator_read()));
        assert_eq!(observer.gate.queue_depth(), QUEUE_PLACES, "the admission took no place");
        assert!(!observer.admits_command(&api, &operator_read()), "the last place was handed out twice");
        observer.withdraw_command(&api, &operator_read());
        assert_eq!(observer.gate.queue_depth(), QUEUE_PLACES - 1);
        assert!(observer.admits_command(&api, &operator_read()), "a withdrawn command kept its place");
    }

    #[test]
    fn work_that_is_never_refused_takes_and_gives_back_no_place() {
        let observer = observer_at_depth(1);
        assert!(observer.admits_command(&meta(Origin::Api, 0), &dali_command()));
        assert!(observer.admits_command(&meta(Origin::Poller, 0), &operator_read()));
        assert_eq!(observer.gate.queue_depth(), 1);
        observer.withdraw_command(&meta(Origin::Poller, 0), &operator_read());
        observer.withdraw_command(&meta(Origin::Api, 0), &dali_command());
        assert_eq!(observer.gate.queue_depth(), 1);
    }

    #[test]
    fn a_withdrawal_from_an_empty_queue_stays_at_zero() {
        let observer = observer_at_depth(0);
        observer.withdraw_command(&meta(Origin::Api, 0), &operator_read());
        assert_eq!(observer.gate.queue_depth(), 0);
    }

    #[test]
    fn another_adapters_command_is_not_ours_to_refuse() {
        let full = observer_at_depth(usize::MAX);
        assert!(full.admits_command(&meta(Origin::Api, 1), &operator_read()));
        assert!(full.admits_command(&meta(Origin::Api, 0), &projector_runtime_update()));
    }

    #[test]
    fn granularity_matches_the_transactional_shape_of_each_kind() {
        assert_eq!(
            yield_granularity(READ_ATTRIBUTES),
            Some(YieldGranularity::Frame)
        );
        assert_eq!(
            yield_granularity("DaliWriteAttributesCommand"),
            Some(YieldGranularity::Step)
        );
        assert_eq!(
            yield_granularity("DaliDiscoverDevicesCommand"),
            Some(YieldGranularity::Never)
        );
        assert_eq!(
            yield_granularity("DaliBusHealthProbeCommand"),
            Some(YieldGranularity::Frame),
            "the probe's broadcast queries are independent frames"
        );
    }
}
