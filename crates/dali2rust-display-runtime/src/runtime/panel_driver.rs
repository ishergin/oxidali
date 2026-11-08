use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use dali2rust_contracts::msg::DisplayPage;
use dali2rust_domain::registry::DisplaySettingsView;

use crate::panel::{DisplayPanel, PanelSetup};
use crate::screen::{render_row, shows_alarm, ScreenRow, ScreenState, SCREEN_ROWS, SPARK_COLS};

pub const PAGE_DWELL: Duration = Duration::from_secs(5);

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x1000_0000_01b3;

pub enum HardwareDisplay {
    None,
    Panel(Arc<Mutex<dyn DisplayPanel>>),
}

impl HardwareDisplay {
    pub fn none() -> Self {
        Self::None
    }

    pub fn panel(panel: Arc<Mutex<dyn DisplayPanel>>) -> Self {
        Self::Panel(panel)
    }

    fn with_panel(&self, f: impl FnOnce(&mut dyn DisplayPanel) -> bool) -> bool {
        match self {
            Self::None => true,
            Self::Panel(panel) => f(&mut *panel.lock().unwrap_or_else(PoisonError::into_inner)),
        }
    }
}

pub(crate) struct PanelDriver {
    applied: Option<PanelSetup>,
    refused: Option<PanelSetup>,
    powered: Option<bool>,
    drawn: [u64; SCREEN_ROWS],
    page: DisplayPage,
    page_since: Instant,
    settings: Option<DisplaySettingsView>,
    activity_seen: u32,
    active_at: Instant,
}

impl PanelDriver {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            applied: None,
            refused: None,
            powered: None,
            drawn: [0; SCREEN_ROWS],
            page: DisplayPage::default(),
            page_since: now,
            settings: None,
            activity_seen: 0,
            active_at: now,
        }
    }

    pub(crate) fn turn_page(&mut self, pages_mask: u8, now: Instant) -> DisplayPage {
        if pages_mask & self.page.mask_bit() == 0 {
            self.show_page(first_page(pages_mask), now);
        } else if now.duration_since(self.page_since) >= PAGE_DWELL {
            self.show_page(next_page(self.page, pages_mask), now);
        }
        self.page
    }

    fn show_page(&mut self, page: DisplayPage, now: Instant) {
        self.page = page;
        self.page_since = now;
    }

    pub(crate) fn drive(
        &mut self,
        hardware: &HardwareDisplay,
        state: &ScreenState,
        activity: u32,
        now: Instant,
    ) {
        let settings = state.sample.display;
        self.note_settings(settings, now);
        if !self.set_up(hardware, panel_setup(settings)) {
            return;
        }
        let awake = settings.enabled && self.is_awake(state, activity, settings, now);
        if awake {
            self.draw(hardware, state);
        }
        self.power(hardware, awake);
    }

    fn note_settings(&mut self, settings: DisplaySettingsView, now: Instant) {
        if self.settings != Some(settings) {
            self.settings = Some(settings);
            self.active_at = now;
        }
    }

    fn is_awake(
        &mut self,
        state: &ScreenState,
        activity: u32,
        settings: DisplaySettingsView,
        now: Instant,
    ) -> bool {
        if activity != self.activity_seen || shows_alarm(state) {
            self.activity_seen = activity;
            self.active_at = now;
        }
        let timeout = Duration::from_secs(u64::from(settings.screensaver_timeout_s));
        settings.screensaver_timeout_s == DisplaySettingsView::SCREENSAVER_OFF
            || now.duration_since(self.active_at) < timeout
    }

    fn set_up(&mut self, hardware: &HardwareDisplay, setup: PanelSetup) -> bool {
        if self.applied == Some(setup) {
            return true;
        }
        if hardware.with_panel(|panel| panel.configure(setup)) {
            self.applied = Some(setup);
            self.refused = None;
            self.powered = Some(false);
            self.drawn = [0; SCREEN_ROWS];
            return true;
        }
        if self.refused != Some(setup) {
            log::warn!("display: no panel answers at 0x{:02X}", setup.i2c_address);
            self.refused = Some(setup);
        }
        false
    }

    fn power(&mut self, hardware: &HardwareDisplay, on: bool) {
        if self.powered == Some(on) {
            return;
        }
        self.powered = hardware.with_panel(|panel| panel.power(on)).then_some(on);
    }

    fn draw(&mut self, hardware: &HardwareDisplay, state: &ScreenState) {
        for (row, drawn) in self.drawn.iter_mut().enumerate() {
            let line = render_row(state, row);
            let digest = digest_row(&line, &state.spark);
            if *drawn != digest && hardware.with_panel(|panel| panel.draw_row(row, &line, &state.spark)) {
                *drawn = digest;
            }
        }
    }
}

fn panel_setup(settings: DisplaySettingsView) -> PanelSetup {
    PanelSetup { i2c_address: settings.i2c_address, upside_down: settings.upside_down }
}

fn first_page(pages_mask: u8) -> DisplayPage {
    DisplayPage::in_mask(pages_mask).next().unwrap_or_default()
}

fn next_page(page: DisplayPage, pages_mask: u8) -> DisplayPage {
    DisplayPage::in_mask(pages_mask)
        .find(|candidate| (*candidate as u8) > (page as u8))
        .unwrap_or_else(|| first_page(pages_mask))
}

fn digest_row(line: &ScreenRow, spark: &[u8; SPARK_COLS]) -> u64 {
    let mut h = FNV_OFFSET;
    let mut mix = |value: u64| h = (h ^ value).wrapping_mul(FNV_PRIME);
    for byte in line.text.as_bytes() {
        mix(u64::from(*byte));
    }
    mix(u64::from(line.invert));
    mix(u64::from(line.bar.unwrap_or(u16::MAX)));
    if line.spark {
        for column in spark {
            mix(u64::from(*column));
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::RecordingPanel;
    use crate::source::DisplaySample;

    const TIMEOUT_S: u32 = 10;

    fn mask(pages: &[DisplayPage]) -> u8 {
        pages.iter().fold(0, |acc, page| acc | page.mask_bit())
    }

    fn quiet_state(display: DisplaySettingsView) -> ScreenState {
        ScreenState {
            ip: Some([10, 0, 0, 2]),
            sample: DisplaySample { adapter_enabled: true, display, ..DisplaySample::default() },
            ..ScreenState::default()
        }
    }

    fn recorder() -> (Arc<Mutex<RecordingPanel>>, HardwareDisplay) {
        let panel = Arc::new(Mutex::new(RecordingPanel::default()));
        let hardware = HardwareDisplay::panel(panel.clone());
        (panel, hardware)
    }

    fn powered(panel: &Arc<Mutex<RecordingPanel>>) -> bool {
        panel.lock().expect("panel").powered
    }

    fn saver(timeout_s: u32) -> DisplaySettingsView {
        DisplaySettingsView { screensaver_timeout_s: timeout_s, ..DisplaySettingsView::default() }
    }

    #[test]
    fn a_quiet_panel_sleeps_after_the_timeout_and_wakes_on_activity() {
        let (panel, hardware) = recorder();
        let state = quiet_state(saver(TIMEOUT_S));
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut driver = PanelDriver::new(start);
        driver.drive(&hardware, &state, 0, at(0));
        assert!(powered(&panel));
        driver.drive(&hardware, &state, 0, at(u64::from(TIMEOUT_S) - 1));
        assert!(powered(&panel), "still inside the timeout");
        driver.drive(&hardware, &state, 0, at(u64::from(TIMEOUT_S)));
        assert!(!powered(&panel), "the timeout ran out with nothing happening");
        driver.drive(&hardware, &state, 1, at(u64::from(TIMEOUT_S) + 1));
        assert!(powered(&panel), "new activity wakes the panel");
    }

    #[test]
    fn a_screensaver_of_zero_never_sleeps() {
        let (panel, hardware) = recorder();
        let state = quiet_state(saver(DisplaySettingsView::SCREENSAVER_OFF));
        let start = Instant::now();
        let mut driver = PanelDriver::new(start);
        driver.drive(&hardware, &state, 0, start);
        driver.drive(&hardware, &state, 0, start + Duration::from_secs(86_400));
        assert!(powered(&panel));
    }

    #[test]
    fn an_alarm_keeps_the_panel_awake() {
        let (panel, hardware) = recorder();
        let mut state = quiet_state(saver(TIMEOUT_S));
        state.ip = None;
        let start = Instant::now();
        let mut driver = PanelDriver::new(start);
        driver.drive(&hardware, &state, 0, start);
        driver.drive(&hardware, &state, 0, start + Duration::from_secs(u64::from(TIMEOUT_S) * 6));
        assert!(powered(&panel), "a lost link is an alarm and the alarm stays on screen");
    }

    #[test]
    fn a_settings_change_counts_as_activity() {
        let (panel, hardware) = recorder();
        let start = Instant::now();
        let mut driver = PanelDriver::new(start);
        driver.drive(&hardware, &quiet_state(saver(TIMEOUT_S)), 0, start);
        let late = start + Duration::from_secs(u64::from(TIMEOUT_S) * 2);
        driver.drive(&hardware, &quiet_state(saver(TIMEOUT_S)), 0, late);
        assert!(!powered(&panel));
        driver.drive(&hardware, &quiet_state(saver(TIMEOUT_S * 2)), 0, late);
        assert!(powered(&panel));
    }

    #[test]
    fn a_new_address_reconfigures_the_panel_and_redraws_every_row() {
        let (panel, hardware) = recorder();
        let start = Instant::now();
        let mut driver = PanelDriver::new(start);
        driver.drive(&hardware, &quiet_state(DisplaySettingsView::default()), 0, start);
        let moved = DisplaySettingsView {
            i2c_address: DisplaySettingsView::I2C_ADDRESSES[1],
            upside_down: true,
            ..DisplaySettingsView::default()
        };
        driver.drive(&hardware, &quiet_state(moved), 0, start);
        let shown = panel.lock().expect("panel");
        assert_eq!(shown.configured, 2);
        assert_eq!(
            shown.setup,
            Some(PanelSetup { i2c_address: DisplaySettingsView::I2C_ADDRESSES[1], upside_down: true })
        );
        assert!(shown.powered && shown.rows.iter().all(|row| !row.is_empty()));
    }

    #[test]
    fn a_disabled_display_is_set_up_and_left_dark() {
        let (panel, hardware) = recorder();
        let off = DisplaySettingsView { enabled: false, ..DisplaySettingsView::default() };
        let start = Instant::now();
        let mut driver = PanelDriver::new(start);
        driver.drive(&hardware, &quiet_state(off), 0, start);
        let shown = panel.lock().expect("panel");
        assert_eq!(shown.configured, 1, "set up, so a panel left lit by a reset goes dark");
        assert!(!shown.powered);
        assert!(shown.rows.iter().all(String::is_empty));
    }

    #[derive(Default)]
    struct SilentPanel {
        configure_calls: u32,
        other_calls: u32,
    }

    impl DisplayPanel for SilentPanel {
        fn configure(&mut self, _setup: PanelSetup) -> bool {
            self.configure_calls += 1;
            false
        }

        fn power(&mut self, _on: bool) -> bool {
            self.other_calls += 1;
            true
        }

        fn draw_row(&mut self, _row: usize, _line: &ScreenRow, _spark: &[u8; SPARK_COLS]) -> bool {
            self.other_calls += 1;
            true
        }
    }

    #[test]
    fn a_panel_that_does_not_answer_gets_set_up_again_and_nothing_else() {
        let panel = Arc::new(Mutex::new(SilentPanel::default()));
        let hardware = HardwareDisplay::panel(panel.clone());
        let start = Instant::now();
        let mut driver = PanelDriver::new(start);
        let state = quiet_state(DisplaySettingsView::default());
        driver.drive(&hardware, &state, 0, start);
        driver.drive(&hardware, &state, 0, start + PAGE_DWELL);
        let silent = panel.lock().expect("panel");
        assert_eq!(silent.configure_calls, 2, "each refresh tries the set-up again");
        assert_eq!(silent.other_calls, 0, "nothing is drawn or powered on a panel never set up");
    }

    #[test]
    fn the_next_page_wraps_to_the_first_selected_one() {
        let selected = mask(&[DisplayPage::Summary, DisplayPage::Bus]);
        assert_eq!(next_page(DisplayPage::Summary, selected), DisplayPage::Bus);
        assert_eq!(next_page(DisplayPage::Bus, selected), DisplayPage::Summary);
        assert_eq!(next_page(DisplayPage::Network, selected), DisplayPage::Bus);
    }

    #[test]
    fn a_single_page_never_turns() {
        let selected = mask(&[DisplayPage::Activity]);
        assert_eq!(next_page(DisplayPage::Activity, selected), DisplayPage::Activity);
        assert_eq!(first_page(selected), DisplayPage::Activity);
    }

    #[test]
    fn a_page_left_out_of_the_selection_gives_way_at_once() {
        let start = Instant::now();
        let mut driver = PanelDriver::new(start);
        assert_eq!(driver.turn_page(mask(&[DisplayPage::Network]), start), DisplayPage::Network);
        let both = mask(&[DisplayPage::Network, DisplayPage::Activity]);
        assert_eq!(driver.turn_page(both, start + PAGE_DWELL / 2), DisplayPage::Network);
        assert_eq!(driver.turn_page(both, start + PAGE_DWELL), DisplayPage::Activity);
    }
}
