use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeReadSection {
    Identity,
    RuntimeStatus,
    Common102,
    Dt8Color,
    Dt6Led,
    Groups,
    Scenes,
    Extended,
    MemoryBanks,
    SceneColours,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttributeReadOutcomes {
    pub identity: AttributeGroupReadOutcome,
    pub runtime_status: AttributeGroupReadOutcome,
    pub common_102: AttributeGroupReadOutcome,
    pub dt8_color: AttributeGroupReadOutcome,
    pub dt6_led: AttributeGroupReadOutcome,
    pub groups: AttributeGroupReadOutcome,
    pub scenes: AttributeGroupReadOutcome,
    pub extended: AttributeGroupReadOutcome,
    pub memory_banks: AttributeGroupReadOutcome,
    pub scene_colours: AttributeGroupReadOutcome,
}

impl AttributeReadOutcomes {
    pub fn for_request(groups: &[DaliAttributeGroup], memory_banks_requested: bool) -> Self {
        let requested = |group: DaliAttributeGroup| {
            if wants_attr_group(groups, group) {
                AttributeGroupReadOutcome::NotAttempted
            } else {
                AttributeGroupReadOutcome::NotRequested
            }
        };
        Self {
            identity: AttributeGroupReadOutcome::NotAttempted,
            runtime_status: requested(DaliAttributeGroup::RuntimeStatus),
            common_102: requested(DaliAttributeGroup::Common102),
            dt8_color: requested(DaliAttributeGroup::Dt8Color),
            dt6_led: requested(DaliAttributeGroup::Dt6Led),
            groups: requested(DaliAttributeGroup::Groups),
            scenes: requested(DaliAttributeGroup::Scenes),
            extended: requested(DaliAttributeGroup::Extended),
            scene_colours: requested(DaliAttributeGroup::SceneColours),
            memory_banks: if memory_banks_requested {
                AttributeGroupReadOutcome::NotAttempted
            } else {
                AttributeGroupReadOutcome::NotRequested
            },
        }
    }

    pub fn record(&mut self, section: AttributeReadSection, outcome: AttributeGroupReadOutcome) {
        let slot = match section {
            AttributeReadSection::Identity => &mut self.identity,
            AttributeReadSection::RuntimeStatus => &mut self.runtime_status,
            AttributeReadSection::Common102 => &mut self.common_102,
            AttributeReadSection::Dt8Color => &mut self.dt8_color,
            AttributeReadSection::Dt6Led => &mut self.dt6_led,
            AttributeReadSection::Groups => &mut self.groups,
            AttributeReadSection::Scenes => &mut self.scenes,
            AttributeReadSection::Extended => &mut self.extended,
            AttributeReadSection::MemoryBanks => &mut self.memory_banks,
            AttributeReadSection::SceneColours => &mut self.scene_colours,
        };
        *slot = outcome;
    }
}

pub fn classify_read_abort(error: SemanticDaliError) -> AttributeGroupReadOutcome {
    if error.is_preempted() {
        return AttributeGroupReadOutcome::Preempted;
    }
    match error {
        SemanticDaliError::OperationFailed("bus_contended") => {
            AttributeGroupReadOutcome::ContendedAbort
        }
        SemanticDaliError::OperationFailed(
            crate::runtime::executor::discovery::DEVICE_TYPE_ENUM_INCOMPLETE,
        ) => AttributeGroupReadOutcome::SequenceIncomplete,
        _ if error.is_device_absent() => AttributeGroupReadOutcome::DeviceAbsent,
        _ => AttributeGroupReadOutcome::TransportAbort,
    }
}

pub(super) fn track<T>(
    outcomes: &mut AttributeReadOutcomes,
    section: AttributeReadSection,
    run: impl FnOnce() -> Result<T, SemanticDaliError>,
) -> Result<T, SemanticDaliError> {
    match run() {
        Ok(value) => {
            outcomes.record(section, AttributeGroupReadOutcome::Success);
            Ok(value)
        }
        Err(error) => {
            outcomes.record(section, classify_read_abort(error));
            Err(error)
        }
    }
}

pub(super) fn track_gated<T>(
    gate: bool,
    outcomes: &mut AttributeReadOutcomes,
    section: AttributeReadSection,
    run: impl FnOnce() -> Result<T, SemanticDaliError>,
) -> Result<Option<T>, SemanticDaliError> {
    if !gate {
        return Ok(None);
    }
    track(outcomes, section, run).map(Some)
}
