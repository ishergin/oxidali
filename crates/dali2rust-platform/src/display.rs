pub trait DisplayDriver {
    type Error: core::fmt::Debug;

    fn init(&mut self) -> Result<(), Self::Error>;
    fn clear(&mut self) -> Result<(), Self::Error>;
    fn flush(&mut self) -> Result<(), Self::Error>;

    fn draw_text(
        &mut self,
        x: u32,
        y: u32,
        text: &str,
        font_size: FontSize,
    ) -> Result<(), Self::Error>;

    fn draw_bitmap(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<(), Self::Error>;

    fn clear_region(&mut self, x: u32, y: u32, width: u32, height: u32) -> Result<(), Self::Error>;

    fn fill_region(&mut self, x: u32, y: u32, width: u32, height: u32) -> Result<(), Self::Error>;

    fn invert_region(&mut self, x: u32, y: u32, width: u32, height: u32)
        -> Result<(), Self::Error>;

    fn flush_region(&mut self, y: u32, height: u32) -> Result<(), Self::Error>;

    fn dimensions(&self) -> (u32, u32);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontSize {
    Small,
    Medium,
}

impl FontSize {
    pub fn char_size(self) -> (u32, u32) {
        match self {
            FontSize::Small => (6, 8),
            FontSize::Medium => (8, 16),
        }
    }
}
