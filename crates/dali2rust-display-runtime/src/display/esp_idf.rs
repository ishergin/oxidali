use dali2rust_platform::display::{DisplayDriver, FontSize};

use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::sys::EspError;

use crate::display::{fonts, ssd1306};
use crate::panel::{DisplayPanel, PanelSetup};
use crate::screen::{ScreenRow, SPARK_COLS, SPARK_H};

const WIDTH: usize = 128;
const HEIGHT: usize = 64;
const PAGES: usize = HEIGHT / 8;
const BUF_SIZE: usize = WIDTH * PAGES;
const I2C_TIMEOUT_MS: u32 = 100;
const ROW_PITCH: u32 = 9;
const ROW_H: u32 = 9;
const BAR_W: u32 = 54;
const BAR_H: u32 = 8;
const BAR_BORDER: u32 = 1;
const BAR_INSET: u32 = 2;
const PERMILLE_FULL: u32 = 1000;
const SPARK_X: u32 = WIDTH as u32 - SPARK_COLS as u32;

#[derive(Debug)]
#[allow(dead_code, reason = "inner error is accessed only via Debug/Display, not directly")]
pub struct DisplayError(EspError);

impl From<EspError> for DisplayError {
    fn from(e: EspError) -> Self {
        DisplayError(e)
    }
}

pub struct Ssd1306Display<'a> {
    i2c: I2cDriver<'a>,
    address: u8,
    upside_down: bool,
    buffer: [u8; BUF_SIZE],
    page_buf: [u8; 1 + WIDTH],
}

impl<'a> Ssd1306Display<'a> {
    pub fn new(i2c: I2cDriver<'a>) -> Self {
        Self {
            i2c,
            address: dali2rust_domain::registry::DisplaySettingsView::I2C_ADDRESSES[0],
            upside_down: false,
            buffer: [0u8; BUF_SIZE],
            page_buf: [0x40u8; 1 + WIDTH],
        }
    }

    fn send_commands(&mut self, cmds: &[u8]) -> Result<(), DisplayError> {
        let mut buf = [0u8; 64];
        if cmds.len() + 1 > buf.len() {
            log::error!(
                "SSD1306: command buffer overflow, {} commands + 1 > {}",
                cmds.len(),
                buf.len()
            );
            return Ok(());
        }
        buf[0] = 0x00;
        buf[1..=cmds.len()].copy_from_slice(cmds);
        self.i2c
            .write(self.address, &buf[..=cmds.len()], I2C_TIMEOUT_MS)?;
        Ok(())
    }

    fn send_page_data(&mut self, page: usize) -> Result<(), DisplayError> {
        self.send_commands(&[0xB0 | (page as u8), 0x00, 0x10])?;

        self.page_buf[1..].copy_from_slice(&self.buffer[page * WIDTH..(page + 1) * WIDTH]);
        self.i2c.write(self.address, &self.page_buf, I2C_TIMEOUT_MS)?;
        Ok(())
    }

    fn pixel(&self, x: u32, y: u32) -> bool {
        if x >= WIDTH as u32 || y >= HEIGHT as u32 {
            return false;
        }
        let idx = (y / 8) as usize * WIDTH + x as usize;
        self.buffer[idx] & (1 << (y % 8)) != 0
    }

    fn set_pixel(&mut self, x: u32, y: u32, on: bool) {
        if x >= WIDTH as u32 || y >= HEIGHT as u32 {
            return;
        }
        let page = (y / 8) as usize;
        let bit = (y % 8) as u8;
        let idx = page * WIDTH + x as usize;
        if on {
            self.buffer[idx] |= 1 << bit;
        } else {
            self.buffer[idx] &= !(1 << bit);
        }
    }

    fn draw_char_5x8(&mut self, x: u32, y: u32, code: u32) -> u32 {
        let glyph = match fonts::lookup_glyph_5x8(code) {
            Some(g) => g,
            None => fonts::lookup_glyph_5x8(b'?' as u32).unwrap(),
        };
        for (col, &byte) in glyph.iter().enumerate() {
            for bit in 0u32..8 {
                if byte & (1 << bit) != 0 {
                    self.set_pixel(x + col as u32, y + bit, true);
                }
            }
        }
        6
    }

    fn draw_char_8x16(&mut self, x: u32, y: u32, code: u32) -> u32 {
        let glyph = match fonts::lookup_glyph_5x8(code) {
            Some(g) => g,
            None => fonts::lookup_glyph_5x8(b'?' as u32).unwrap(),
        };
        for (col, &byte) in glyph.iter().enumerate() {
            let cx = x + (col * 2) as u32;
            if cx >= WIDTH as u32 {
                break;
            }
            for bit in 0u32..8 {
                if byte & (1 << bit) != 0 {
                    self.set_pixel(cx, y + bit * 2, true);
                    self.set_pixel(cx, y + bit * 2 + 1, true);
                    self.set_pixel(cx + 1, y + bit * 2, true);
                    self.set_pixel(cx + 1, y + bit * 2 + 1, true);
                }
            }
        }
        12
    }
}

impl<'a> DisplayDriver for Ssd1306Display<'a> {
    type Error = DisplayError;

    fn init(&mut self) -> Result<(), Self::Error> {
        self.send_commands(&ssd1306::init_sequence(self.upside_down))?;
        self.send_commands(&[ssd1306::power_command(true)])
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.buffer.fill(0);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        for page in 0..PAGES {
            self.send_page_data(page)?;
        }
        Ok(())
    }

    fn draw_text(
        &mut self,
        x: u32,
        y: u32,
        text: &str,
        font_size: FontSize,
    ) -> Result<(), Self::Error> {
        let mut cx = x;
        for ch in text.chars() {
            let advance = match font_size {
                FontSize::Small => self.draw_char_5x8(cx, y, ch as u32),
                FontSize::Medium => self.draw_char_8x16(cx, y, ch as u32),
            };
            cx += advance;
            if cx >= WIDTH as u32 {
                break;
            }
        }
        Ok(())
    }

    fn draw_bitmap(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        data: &[u8],
    ) -> Result<(), Self::Error> {
        let mut bit_idx: usize = 0;
        for row in 0..height {
            for col in 0..width {
                let byte_idx = bit_idx / 8;
                let bit_pos = 7 - (bit_idx % 8);
                let on = byte_idx < data.len() && data[byte_idx] & (1 << bit_pos) != 0;
                self.set_pixel(x + col, y + row, on);
                bit_idx += 1;
            }
        }
        Ok(())
    }

    fn clear_region(&mut self, x: u32, y: u32, width: u32, height: u32) -> Result<(), Self::Error> {
        for dy in 0..height {
            for dx in 0..width {
                self.set_pixel(x + dx, y + dy, false);
            }
        }
        Ok(())
    }

    fn fill_region(&mut self, x: u32, y: u32, width: u32, height: u32) -> Result<(), Self::Error> {
        for dy in 0..height {
            for dx in 0..width {
                self.set_pixel(x + dx, y + dy, true);
            }
        }
        Ok(())
    }

    fn invert_region(
        &mut self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<(), Self::Error> {
        for dy in 0..height {
            for dx in 0..width {
                let lit = self.pixel(x + dx, y + dy);
                self.set_pixel(x + dx, y + dy, !lit);
            }
        }
        Ok(())
    }

    fn flush_region(&mut self, y: u32, height: u32) -> Result<(), Self::Error> {
        if height == 0 {
            return Ok(());
        }
        let first = (y as usize / 8).min(PAGES - 1);
        let last = ((y + height - 1) as usize / 8).min(PAGES - 1);
        for page in first..=last {
            self.send_page_data(page)?;
        }
        Ok(())
    }

    fn dimensions(&self) -> (u32, u32) {
        (WIDTH as u32, HEIGHT as u32)
    }
}

impl Ssd1306Display<'static> {
    fn draw_bar(&mut self, y: u32, permille: u16) {
        let x = WIDTH as u32 - BAR_W;
        let _ = self.fill_region(x, y, BAR_W, BAR_BORDER);
        let _ = self.fill_region(x, y + BAR_H - BAR_BORDER, BAR_W, BAR_BORDER);
        let _ = self.fill_region(x, y, BAR_BORDER, BAR_H);
        let _ = self.fill_region(WIDTH as u32 - BAR_BORDER, y, BAR_BORDER, BAR_H);
        let filled = u32::from(permille) * (BAR_W - 2 * BAR_INSET) / PERMILLE_FULL;
        let _ = self.fill_region(x + BAR_INSET, y + BAR_INSET, filled, BAR_H - 2 * BAR_INSET);
    }

    fn draw_spark(&mut self, y: u32, spark: &[u8; SPARK_COLS]) {
        for (i, h) in spark.iter().enumerate() {
            let h = u32::from(*h);
            if h == 0 {
                continue;
            }
            let top = y + 1 + u32::from(SPARK_H) - h;
            let _ = self.fill_region(SPARK_X + i as u32, top, 1, h);
        }
    }
}

impl DisplayPanel for Ssd1306Display<'static> {
    fn configure(&mut self, setup: PanelSetup) -> bool {
        self.address = setup.i2c_address;
        self.upside_down = setup.upside_down;
        self.buffer.fill(0);
        self.send_commands(&ssd1306::init_sequence(setup.upside_down)).is_ok() && self.flush().is_ok()
    }

    fn power(&mut self, on: bool) -> bool {
        self.send_commands(&[ssd1306::power_command(on)]).is_ok()
    }

    fn draw_row(&mut self, row: usize, line: &ScreenRow, spark: &[u8; SPARK_COLS]) -> bool {
        let y = row as u32 * ROW_PITCH;
        let _ = self.clear_region(0, y, WIDTH as u32, ROW_H);
        let _ = self.draw_text(0, y, &line.text, FontSize::Small);
        if let Some(permille) = line.bar {
            self.draw_bar(y, permille);
        }
        if line.spark {
            self.draw_spark(y, spark);
        }
        if line.invert {
            let _ = self.invert_region(0, y, WIDTH as u32, ROW_H);
        }
        self.flush_region(y, ROW_H).is_ok()
    }
}
