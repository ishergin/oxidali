use dali2rust_contracts::msg::{
    ColorMode, CompactErrorPayload, LastDapcSource, LightSetpoint, RuntimeObservation, RuntimeSource,
};

#[derive(Clone, Debug)]
pub(crate) struct StatusFlagsData {
    pub raw: u8,
    pub lamp_failure: bool,
    pub gear_failure: bool,
    pub lamp_on: bool,
    pub limit_error: bool,
    pub fade_running: bool,
    pub reset_state: bool,
    pub missing_short_address: bool,
    pub power_cycle_seen: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct FailureStatusData {
    pub raw: u8,
    pub lamp_failure: bool,
    pub gear_failure: bool,
    pub communication_failure: bool,
    pub source: RuntimeSource,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeObservationData {
    pub has_observation: bool,
    pub power: dali2rust_contracts::msg::PowerState,
    pub color_mode: ColorMode,
    pub kelvin: Option<u16>,
    pub xy: Option<(f64, f64)>,
    pub rgb: Option<(u8, u8, u8)>,
    pub waf: Option<(u8, u8, u8)>,
    pub status_flags: Option<StatusFlagsData>,
    pub failure_status: Option<FailureStatusData>,
    pub value_source: Option<RuntimeSource>,
    pub last_seen_ms: Option<u64>,
    pub last_dapc_source: LastDapcSource,
    pub error: Option<CompactErrorPayload>,
    pub observed_at_mono_ms: Option<u32>,
}

impl Default for RuntimeObservationData {
    fn default() -> Self {
        Self {
            observed_at_mono_ms: None,
            has_observation: false,
            power: dali2rust_contracts::msg::PowerState::Unknown,
            color_mode: ColorMode::Unknown,
            kelvin: None,
            xy: None,
            rgb: None,
            waf: None,
            status_flags: None,
            failure_status: None,
            value_source: None,
            last_seen_ms: None,
            last_dapc_source: LastDapcSource::Unknown,
            error: None,
        }
    }
}

fn status_flags_data(sf: &dali2rust_contracts::msg::StatusFlags) -> StatusFlagsData {
    StatusFlagsData {
        raw: sf.raw,
        lamp_failure: sf.lamp_failure,
        gear_failure: sf.gear_failure,
        lamp_on: sf.lamp_on,
        limit_error: sf.limit_error,
        fade_running: sf.fade_running,
        reset_state: sf.reset_state,
        missing_short_address: sf.missing_short_address,
        power_cycle_seen: sf.power_cycle_seen,
    }
}

fn failure_status_data(fs: &dali2rust_contracts::msg::FailureStatus) -> FailureStatusData {
    FailureStatusData {
        raw: fs.raw,
        lamp_failure: fs.lamp_failure,
        gear_failure: fs.gear_failure,
        communication_failure: fs.communication_failure,
        source: fs.source,
    }
}

impl RuntimeObservationData {
    pub(crate) fn apply_setpoint(&mut self, setpoint: &LightSetpoint) {
        if setpoint.power != dali2rust_contracts::msg::PowerState::Unknown {
            self.power = setpoint.power;
        }
        if let Some(cv) = setpoint.color.as_ref() {
            self.apply_color(cv);
        }
        self.has_observation = true;
    }

    fn apply_color(&mut self, cv: &dali2rust_contracts::msg::ColorValue) {
        match cv.mode {
            ColorMode::Cct => {
                self.kelvin = Some(cv.color_temperature_kelvin);
                self.xy = None;
                self.clear_rgbwaf();
            }
            ColorMode::Xy => {
                self.kelvin = None;
                self.xy = Some((f64::from(cv.x) / 65535.0, f64::from(cv.y) / 65535.0));
                self.clear_rgbwaf();
            }
            ColorMode::Rgb => {
                self.kelvin = None;
                self.xy = None;
                self.rgb = Some((cv.r, cv.g, cv.b));
                self.waf = None;
            }
            ColorMode::Rgbwaf => {
                self.kelvin = None;
                self.xy = None;
                self.rgb = Some((cv.r, cv.g, cv.b));
                self.waf = Some((cv.w, cv.a, cv.f));
            }
            ColorMode::Brightness => {
                self.kelvin = None;
                self.xy = None;
                self.clear_rgbwaf();
            }
            ColorMode::None | ColorMode::Unknown => return,
        }
        self.color_mode = cv.mode;
    }

    fn clear_rgbwaf(&mut self) {
        self.rgb = None;
        self.waf = None;
    }

    pub(crate) fn apply_observation(&mut self, observation: &RuntimeObservation) {
        self.has_observation = true;
        if let Some(sf) = observation.status_flags.as_ref() {
            self.status_flags = Some(status_flags_data(sf));
        }
        if let Some(fs) = observation.failure_status.as_ref() {
            self.failure_status = Some(failure_status_data(fs));
        }
        if let Some(last_seen_ms) = observation.last_seen_ms {
            self.last_seen_ms = Some(last_seen_ms);
        }
        self.apply_reachability(observation);
    }

    fn apply_reachability(&mut self, observation: &RuntimeObservation) {
        if observation.reports_absence() {
            self.error = observation.error.clone();
        } else if observation.status_flags.is_some() || observation.failure_status.is_some() {
            self.error = None;
        }
    }

    pub(crate) fn merge_command_metadata(
        &mut self,
        observation_value_source: Option<RuntimeSource>,
        entry_last_dapc_source: Option<LastDapcSource>,
        entry_source: RuntimeSource,
        states_value: bool,
    ) {
        if states_value {
            self.value_source = observation_value_source.or(Some(entry_source));
        }
        if let Some(source) = entry_last_dapc_source {
            self.last_dapc_source = source;
        }
    }

    pub(crate) fn observation_adds_nothing(&self, observation: &RuntimeObservation) -> bool {
        observation.status_flags.is_none()
            && observation.failure_status.is_none()
            && observation.last_seen_ms.is_none()
            && observation.error.as_ref().map(|e| e.code) == self.error.as_ref().map(|e| e.code)
    }
}

#[cfg(test)]
mod color_transition_tests {
    use super::*;
    use dali2rust_contracts::msg::{ColorValue, PowerState};

    const CCT: ColorMode = ColorMode::Cct;
    const XY: ColorMode = ColorMode::Xy;
    const RGB: ColorMode = ColorMode::Rgb;
    const RGBWAF: ColorMode = ColorMode::Rgbwaf;
    const BRIGHTNESS: ColorMode = ColorMode::Brightness;

    fn observed(mode: ColorMode) -> ColorValue {
        ColorValue {
            mode,
            color_temperature_kelvin: 4000,
            x: 6553,
            y: 13107,
            r: 254,
            g: 10,
            b: 20,
            w: 0,
            a: 0,
            f: 0,
        }
    }

    fn setpoint(color: Option<ColorValue>) -> LightSetpoint {
        LightSetpoint {
            power: PowerState::On,
            level: 100,
            color,
        }
    }

    fn stored_in(mode: ColorMode) -> RuntimeObservationData {
        let mut data = RuntimeObservationData::default();
        data.apply_setpoint(&setpoint(Some(observed(mode))));
        data
    }

    fn assert_only(data: &RuntimeObservationData, mode: ColorMode) {
        assert_eq!(data.color_mode, mode, "active mode");
        assert_eq!(data.kelvin.is_some(), mode == CCT, "kelvin held for {mode:?}");
        assert_eq!(data.xy.is_some(), mode == XY, "xy held for {mode:?}");
        assert_eq!(
            data.rgb.is_some(),
            mode == RGB || mode == RGBWAF,
            "rgb held for {mode:?}"
        );
    }

    #[test]
    fn every_transition_between_value_modes_keeps_exactly_one_representation() {
        for from in [CCT, XY, RGB, RGBWAF, BRIGHTNESS] {
            for to in [CCT, XY, RGB, RGBWAF, BRIGHTNESS] {
                let mut data = stored_in(from);
                data.apply_setpoint(&setpoint(Some(observed(to))));
                assert_only(&data, to);
            }
        }
    }

    #[test]
    fn brightness_clears_a_previously_stored_colour() {
        let mut data = stored_in(CCT);
        assert_eq!(data.kelvin, Some(4000));
        data.apply_setpoint(&setpoint(Some(observed(BRIGHTNESS))));
        assert_only(&data, BRIGHTNESS);
    }

    #[test]
    fn an_unobserved_colour_keeps_the_stored_one() {
        let mut data = stored_in(CCT);
        data.apply_setpoint(&setpoint(None));
        assert_only(&data, CCT);
        assert_eq!(data.kelvin, Some(4000));
        assert_eq!(data.power, PowerState::On, "power still applies");
    }

    #[test]
    fn a_colour_only_setpoint_keeps_the_stored_power() {
        let mut data = stored_in(CCT);
        assert_eq!(data.power, PowerState::On);
        data.apply_setpoint(&LightSetpoint {
            power: PowerState::Unknown,
            level: 0,
            color: Some(observed(CCT)),
        });
        assert_eq!(data.power, PowerState::On, "no statement, no change");
        data.apply_setpoint(&LightSetpoint {
            power: PowerState::Off,
            level: 0,
            color: None,
        });
        assert_eq!(data.power, PowerState::Off, "an explicit OFF still lands");
    }

    #[test]
    fn a_modeless_colour_erases_nothing() {
        for mode in [ColorMode::None, ColorMode::Unknown] {
            let mut data = stored_in(CCT);
            data.apply_setpoint(&setpoint(Some(observed(mode))));
            assert_only(&data, CCT);
            assert_eq!(data.kelvin, Some(4000), "{mode:?} must not clear kelvin");
        }
    }
}

#[cfg(test)]
mod reachability_tests {
    use super::*;
    use dali2rust_contracts::msg::{ErrorCode, StatusFlags};

    fn absent() -> RuntimeObservation {
        RuntimeObservation {
            status_flags: None,
            failure_status: None,
            value_source: Some(RuntimeSource::Poller),
            last_seen_ms: None,
            last_dapc_source: LastDapcSource::default(),
            error: Some(CompactErrorPayload::new(ErrorCode::DeviceAbsent, "no answer")),
        }
    }

    fn commanded() -> RuntimeObservation {
        RuntimeObservation::timestamped(RuntimeSource::Api, 1_000)
    }

    fn read_back() -> RuntimeObservation {
        let mut o = RuntimeObservation::timestamped(RuntimeSource::Api, 2_000);
        o.status_flags = Some(StatusFlags {
            raw: 0,
            lamp_failure: false,
            gear_failure: false,
            lamp_on: true,
            limit_error: false,
            fade_running: false,
            reset_state: false,
            missing_short_address: false,
            power_cycle_seen: false,
        });
        o
    }

    #[test]
    fn absence_is_set_by_the_observation_that_saw_it() {
        let mut rt = RuntimeObservationData::default();
        rt.apply_observation(&absent());
        assert_eq!(rt.error.as_ref().map(|e| e.code), Some(ErrorCode::DeviceAbsent));
    }

    #[test]
    fn a_command_of_ours_never_clears_it_but_the_gears_own_bytes_do() {
        let mut rt = RuntimeObservationData::default();
        rt.apply_observation(&absent());

        rt.apply_observation(&commanded());
        assert_eq!(
            rt.error.as_ref().map(|e| e.code),
            Some(ErrorCode::DeviceAbsent),
            "a command echo must not clear a reachability fault"
        );

        rt.apply_observation(&read_back());
        assert!(rt.error.is_none(), "the gear's own bytes clear it");
    }
}
