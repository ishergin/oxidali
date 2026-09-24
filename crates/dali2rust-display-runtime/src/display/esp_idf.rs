use dali2rust_platform::display::{DisplayDriver, FontSize};

use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::sys::EspError;

use crate::display::fonts;

const SSD1306_ADDR: u8 = 0x3C;
const WIDTH: usize = 128;
const HEIGHT: usize = 64;
const PAGES: usize = HEIGHT / 8;
const BUF_SIZE: usize = WIDTH * PAGES;
const I2C_TIMEOUT_MS: u32 = 100;

const CMD_DISPLAY_OFF: u8 = 0xAE;
const CMD_DISPLAY_ON: u8 = 0xAF;
const CMD_SET_DISPLAY_CLOCK_DIV: u8 = 0xD5;
const CMD_SET_MULTIPLEX: u8 = 0xA8;
const CMD_SET_DISPLAY_OFFSET: u8 = 0xD3;
const CMD_SET_START_LINE: u8 = 0x40;
const CMD_SET_CHARGE_PUMP: u8 = 0x8D;
const CMD_MEMORY_MODE: u8 = 0x20;
const CMD_SEG_REMAP: u8 = 0xA1;
const CMD_COM_SCAN_DEC: u8 = 0xC8;
const CMD_SET_COM_PINS: u8 = 0xDA;
const CMD_SET_CONTRAST: u8 = 0x81;
const CMD_SET_PRECHARGE: u8 = 0xD9;
const CMD_SET_VCOM_DETECT: u8 = 0xDB;
const CMD_DISPLAY_ALL_ON_RESUME: u8 = 0xA4;
const CMD_NORMAL_DISPLAY: u8 = 0xA6;

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
    buffer: [u8; BUF_SIZE],
    page_buf: [u8; 1 + WIDTH],
}

impl<'a> Ssd1306Display<'a> {
    pub fn new(i2c: I2cDriver<'a>) -> Self {
        Self {
            i2c,
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
            .write(SSD1306_ADDR, &buf[..=cmds.len()], I2C_TIMEOUT_MS)?;
        Ok(())
    }

    fn send_page_data(&mut self, page: usize) -> Result<(), DisplayError> {
        self.send_commands(&[0xB0 | (page as u8), 0x00, 0x10])?;

        self.page_buf[1..].copy_from_slice(&self.buffer[page * WIDTH..(page + 1) * WIDTH]);
        self.i2c.write(SSD1306_ADDR, &self.page_buf, I2C_TIMEOUT_MS)?;
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
        self.send_commands(&[
            CMD_DISPLAY_OFF,
            CMD_SET_DISPLAY_CLOCK_DIV,
            0x80,
            CMD_SET_MULTIPLEX,
            0x3F,
            CMD_SET_DISPLAY_OFFSET,
            0x00,
            CMD_SET_START_LINE,
            CMD_SET_CHARGE_PUMP,
            0x14,
            CMD_MEMORY_MODE,
            0x00,
            CMD_SEG_REMAP,
            CMD_COM_SCAN_DEC,
            CMD_SET_COM_PINS,
            0x12,
            CMD_SET_CONTRAST,
            0xCF,
            CMD_SET_PRECHARGE,
            0xF1,
            CMD_SET_VCOM_DETECT,
            0x40,
            CMD_DISPLAY_ALL_ON_RESUME,
            CMD_NORMAL_DISPLAY,
            CMD_DISPLAY_ON,
        ])?;
        Ok(())
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
