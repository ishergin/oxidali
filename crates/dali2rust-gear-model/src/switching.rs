use dali2rust_domain::dali::devices::dt7_switching::{
    Dt7Command, Threshold, FEATURE_ADJUSTABLE_HOLD_OFF, FEATURE_ADJUSTABLE_THRESHOLDS,
    FEATURE_REFERENCE_SYSTEM_POWER, SWITCH_STATUS_REFERENCE_FAILED,
};

use crate::gear::{Gear, DALI_YES};

// IEC 62386-208 §11.3.4.2
const EXTENDED_VERSION_NUMBER_208: u8 = 1;
// IEC 62386-208 Table 2
const MIN_LEVEL_LOWEST: u8 = 1;
const DEFAULT_ERROR_HOLD_OFF: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switching {
    pub thresholds: [u8; 4],
    pub error_hold_off: u8,
    pub switch_status: u8,
}

impl Switching {
    // IEC 62386-208 Table 2, §11.3.4.1
    pub fn factory(features: u8) -> Self {
        let never_referenced = if features & FEATURE_REFERENCE_SYSTEM_POWER != 0 {
            SWITCH_STATUS_REFERENCE_FAILED
        } else {
            0
        };
        Self {
            thresholds: Threshold::ALL.map(Threshold::factory),
            error_hold_off: DEFAULT_ERROR_HOLD_OFF,
            switch_status: never_referenced,
        }
    }

    pub fn threshold(&self, threshold: Threshold) -> u8 {
        self.thresholds[usize::from(threshold.index())]
    }

    // IEC 62386-208 §11.3.4.1
    pub(crate) fn store(&mut self, features: u8, command: Dt7Command, dtr0: u8) {
        match command {
            Dt7Command::StoreDtrAsThreshold(t) if features & FEATURE_ADJUSTABLE_THRESHOLDS != 0 => {
                self.thresholds[usize::from(t.index())] = t.within_range(dtr0);
            }
            Dt7Command::StoreDtrAsErrorHoldOffTime if features & FEATURE_ADJUSTABLE_HOLD_OFF != 0 => {
                self.error_hold_off = dtr0;
            }
            _ => {}
        }
    }
}

impl Gear {
    // IEC 62386-208 §11.3.4.2
    pub(crate) fn dt7_answer(&self, command: Dt7Command) -> Option<u8> {
        let features = self.spec.switching_features;
        match command {
            Dt7Command::QueryFeatures => Some(features),
            Dt7Command::QuerySwitchStatus => Some(self.switching.switch_status),
            Dt7Command::QueryThreshold(t) => Some(self.switching.threshold(t)),
            Dt7Command::QueryErrorHoldOffTime => Some(self.switching.error_hold_off),
            Dt7Command::QueryGearType => Some(self.spec.switching_gear_type),
            Dt7Command::QueryReferenceMeasurementFailed => {
                (self.switching.switch_status & SWITCH_STATUS_REFERENCE_FAILED != 0).then_some(DALI_YES)
            }
            Dt7Command::QueryExtendedVersionNumber => Some(EXTENDED_VERSION_NUMBER_208),
            Dt7Command::QueryReferenceRunning
            | Dt7Command::ReferenceSystemPower
            | Dt7Command::StoreDtrAsThreshold(_)
            | Dt7Command::StoreDtrAsErrorHoldOffTime => None,
        }
    }

    // IEC 62386-208 §9.4, Table 2
    pub(crate) fn levels_fixed(&self) -> bool {
        self.is_dt7() && self.spec.switching_features & FEATURE_ADJUSTABLE_THRESHOLDS == 0
    }

    pub(crate) fn min_level_floor(&self) -> u8 {
        if self.is_dt7() {
            MIN_LEVEL_LOWEST
        } else {
            self.spec.phm
        }
    }
}
