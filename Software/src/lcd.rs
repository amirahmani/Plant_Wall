//! Minimal driver for a 16x2 HD44780 LCD behind a PCF8574 I2C backpack.
//!
//! Backpack pin mapping (the common blue/green modules):
//!   P0 = RS, P1 = RW, P2 = EN, P3 = backlight, P4..P7 = D4..D7

use core::fmt;
use embedded_hal::{delay::DelayNs, i2c::I2c};

const RS: u8 = 0x01; // register select: 1 = data, 0 = command
const EN: u8 = 0x04; // enable strobe
const BL: u8 = 0x08; // backlight on

pub const COLS: usize = 16;

pub struct Lcd<I> {
    i2c: I,
    addr: u8,
}

impl<I: I2c> Lcd<I> {
    /// Initialise the display in 4-bit, 2-line mode. `addr` is usually 0x27 or 0x3F.
    pub fn new(i2c: I, addr: u8, delay: &mut impl DelayNs) -> Result<Self, I::Error> {
        let mut lcd = Lcd { i2c, addr };

        // Power-on wait, then the datasheet's "reset by instruction" sequence.
        delay.delay_ms(50);
        lcd.write4(0x30, 0, delay)?;
        delay.delay_ms(5);
        lcd.write4(0x30, 0, delay)?;
        delay.delay_us(150);
        lcd.write4(0x30, 0, delay)?;
        lcd.write4(0x20, 0, delay)?; // switch to 4-bit interface

        lcd.command(0x28, delay)?; // 4-bit, 2 lines, 5x8 font
        lcd.command(0x0C, delay)?; // display on, cursor off, blink off
        lcd.command(0x06, delay)?; // cursor moves right after each char
        lcd.clear(delay)?;
        Ok(lcd)
    }

    pub fn clear(&mut self, delay: &mut impl DelayNs) -> Result<(), I::Error> {
        self.command(0x01, delay)?;
        delay.delay_ms(2); // clear is slow
        Ok(())
    }

    pub fn set_cursor(&mut self, col: u8, row: u8, delay: &mut impl DelayNs) -> Result<(), I::Error> {
        let offset = if row == 0 { 0x00 } else { 0x40 };
        self.command(0x80 | (offset + col), delay)
    }

    /// Turn the blinking block cursor on or off (shown at the current cursor position).
    pub fn cursor_blink(&mut self, on: bool, delay: &mut impl DelayNs) -> Result<(), I::Error> {
        // 0x0D = display on, cursor off, blink on; 0x0C = display on, no cursor
        self.command(if on { 0x0D } else { 0x0C }, delay)
    }

    /// Write a full 16-character line, padded with spaces so old text is overwritten.
    /// Use with `format_args!`, e.g. `lcd.write_line(1, format_args!("R{} P{}", r, p), &mut delay)`.
    ///
    /// The cursor command and all 16 characters go out in ONE I2C transaction
    /// (~6 ms at 100 kHz instead of ~15 ms byte-by-byte), so a line fits in a 10 ms task.
    /// The PCF8574 updates its outputs after every byte, and each byte takes ~90 us,
    /// which is plenty for the LCD's enable pulse and 37 us per-character time.
    pub fn write_line(
        &mut self,
        row: u8,
        args: fmt::Arguments,
        delay: &mut impl DelayNs,
    ) -> Result<(), I::Error> {
        let mut line = LineBuf { buf: [b' '; COLS], len: 0 };
        let _ = fmt::write(&mut line, args); // text past 16 chars is cut off

        let mut out = [0u8; 4 * (COLS + 1)];
        let cursor_cmd = 0x80 | if row == 0 { 0x00 } else { 0x40 };
        out[0..4].copy_from_slice(&frame(cursor_cmd, 0));
        for (i, &b) in line.buf.iter().enumerate() {
            out[4 * (i + 1)..4 * (i + 2)].copy_from_slice(&frame(b, RS));
        }
        self.i2c.write(self.addr, &out)?;
        delay.delay_us(50);
        Ok(())
    }

    fn command(&mut self, cmd: u8, delay: &mut impl DelayNs) -> Result<(), I::Error> {
        self.send(cmd, 0, delay)
    }

    /// Send one byte as two 4-bit nibbles, high nibble first.
    fn send(&mut self, byte: u8, mode: u8, delay: &mut impl DelayNs) -> Result<(), I::Error> {
        self.write4(byte & 0xF0, mode, delay)?;
        self.write4((byte << 4) & 0xF0, mode, delay)
    }

    /// Put a nibble on D4..D7 and pulse EN to latch it.
    fn write4(&mut self, nibble: u8, mode: u8, delay: &mut impl DelayNs) -> Result<(), I::Error> {
        let data = nibble | mode | BL;
        self.i2c.write(self.addr, &[data | EN])?;
        delay.delay_us(1);
        self.i2c.write(self.addr, &[data])?;
        delay.delay_us(50);
        Ok(())
    }
}

/// One byte as the 4 backpack writes that clock it into the LCD:
/// high nibble with EN high, EN low, then the low nibble the same way.
fn frame(byte: u8, mode: u8) -> [u8; 4] {
    let hi = (byte & 0xF0) | mode | BL;
    let lo = ((byte << 4) & 0xF0) | mode | BL;
    [hi | EN, hi, lo | EN, lo]
}

/// Fixed 16-byte buffer that `format_args!` can write into, no allocator needed.
struct LineBuf {
    buf: [u8; COLS],
    len: usize,
}

impl fmt::Write for LineBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if self.len < COLS {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}
