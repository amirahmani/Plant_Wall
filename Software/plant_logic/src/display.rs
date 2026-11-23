//! Building LCD text without an allocator (the LCD driver itself is in firmware/src/lcd.rs).

use crate::layout::{cal_entry, MAX_PLANTS, PLANTS_PER_ROW, ROWS, SENSORS};
use crate::sensors::{ADC_MAX, ADC_VREF};

/// Small fixed text buffer for building an LCD line without an allocator.
pub struct Text {
    buf: [u8; 16],
    len: usize,
}

impl Text {
    pub fn new() -> Self {
        Text { buf: [b' '; 16], len: 0 }
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
    }
}

impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for b in s.bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

/// One CALIBRATE line, e.g. "R1P1 1.1V LOW HI", or "R1P3 ---- --- --" if not detected.
/// `just_set` shows "SET!" in place of the voltage for a moment after a capture.
pub fn cal_text(i: usize, detected: &[bool; SENSORS], raw: &[u16; SENSORS], just_set: bool) -> Text {
    use core::fmt::Write;
    let mut t = Text::new();
    if let Some((r, p, s)) = cal_entry(i) {
        if !detected[s] {
            let _ = write!(t, "R{}P{} ---- --- --", r + 1, p + 1);
        } else if just_set {
            let _ = write!(t, "R{}P{} SET! LOW HI", r + 1, p + 1);
        } else {
            let volts = raw[s] as f32 * ADC_VREF / ADC_MAX;
            let _ = write!(t, "R{}P{} {:.1}V LOW HI", r + 1, p + 1, volts);
        }
    }
    t
}

/// Keep the selected line visible in the 2-line window.
pub fn cal_scroll(sel: usize, top: usize) -> usize {
    if sel < top {
        sel
    } else if sel > top + 1 {
        sel - 1
    } else {
        top
    }
}

/// One character per plant, rows separated by '|', e.g. "111|1-1|---".
/// Returned as a fixed buffer + length so it works without an allocator.
pub fn plant_map(f: impl Fn(usize) -> u8) -> ([u8; 16], usize) {
    let mut buf = [b' '; 16];
    let mut len = 0;
    for r in 0..ROWS {
        if r > 0 && len < buf.len() {
            buf[len] = b'|';
            len += 1;
        }
        for p in 0..PLANTS_PER_ROW[r] {
            if len < buf.len() {
                buf[len] = f(r * MAX_PLANTS + p);
                len += 1;
            }
        }
    }
    (buf, len)
}

/// Seconds since midnight -> (hours, minutes)
pub fn hh_mm(tod: u32) -> (u8, u8) {
    ((tod / 3600) as u8, ((tod / 60) % 60) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    #[test]
    fn text_is_cut_at_16_chars() {
        let mut t = Text::new();
        let _ = write!(t, "0123456789ABCDEFGHIJ");
        assert_eq!(t.as_str(), "0123456789ABCDEF");
    }

    #[test]
    fn cal_line_shows_volts() {
        let mut raw = [0u16; SENSORS];
        raw[0] = 1365; // 1365 / 4095 * 3.3 = 1.1 V
        let t = cal_text(0, &[true; SENSORS], &raw, false);
        assert_eq!(t.as_str(), "R1P1 1.1V LOW HI");
    }

    #[test]
    fn cal_line_for_missing_sensor() {
        let t = cal_text(0, &[false; SENSORS], &[0; SENSORS], false);
        assert_eq!(t.as_str(), "R1P1 ---- --- --");
    }

    #[test]
    fn cal_line_after_capture() {
        let t = cal_text(0, &[true; SENSORS], &[0; SENSORS], true);
        assert_eq!(t.as_str(), "R1P1 SET! LOW HI");
    }

    #[test]
    fn cal_scroll_keeps_selection_visible() {
        assert_eq!(cal_scroll(0, 0), 0); // first line, no scroll
        assert_eq!(cal_scroll(1, 0), 0); // second line still fits
        assert_eq!(cal_scroll(2, 0), 1); // moving down scrolls by one
        assert_eq!(cal_scroll(0, 1), 0); // moving up past the top scrolls back
    }

    #[test]
    fn plant_map_separates_rows() {
        // Written for the 2 x 3 wall: update the expected text if the layout changes
        let (buf, len) = plant_map(|i| b'0' + i as u8);
        assert_eq!(core::str::from_utf8(&buf[..len]).unwrap(), "012|345");
    }

    #[test]
    fn hh_mm_examples() {
        assert_eq!(hh_mm(0), (0, 0));
        assert_eq!(hh_mm(7 * 3600 + 30 * 60 + 59), (7, 30));
        assert_eq!(hh_mm(86_400 - 1), (23, 59));
    }
}
