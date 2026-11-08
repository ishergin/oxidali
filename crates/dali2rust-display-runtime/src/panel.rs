use crate::screen::{ScreenRow, SCREEN_ROWS, SPARK_COLS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelSetup {
    pub i2c_address: u8,
    pub upside_down: bool,
}

pub trait DisplayPanel: Send {
    fn configure(&mut self, setup: PanelSetup) -> bool;

    fn power(&mut self, on: bool) -> bool;

    fn draw_row(&mut self, row: usize, line: &ScreenRow, spark: &[u8; SPARK_COLS]) -> bool;
}

#[derive(Debug, Default)]
pub struct RecordingPanel {
    pub setup: Option<PanelSetup>,
    pub configured: u32,
    pub powered: bool,
    pub rows: [String; SCREEN_ROWS],
}

impl RecordingPanel {
    pub fn shows_row_starting_with(&self, prefix: &str) -> bool {
        self.powered && self.rows.iter().any(|row| row.starts_with(prefix))
    }
}

impl DisplayPanel for RecordingPanel {
    fn configure(&mut self, setup: PanelSetup) -> bool {
        self.setup = Some(setup);
        self.configured = self.configured.wrapping_add(1);
        self.powered = false;
        self.rows = Default::default();
        true
    }

    fn power(&mut self, on: bool) -> bool {
        self.powered = on;
        true
    }

    fn draw_row(&mut self, row: usize, line: &ScreenRow, _spark: &[u8; SPARK_COLS]) -> bool {
        let Some(slot) = self.rows.get_mut(row) else {
            return false;
        };
        line.text.as_str().clone_into(slot);
        true
    }
}
