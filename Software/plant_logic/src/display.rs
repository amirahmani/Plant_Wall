//! Building LCD text without an allocator (the LCD driver itself is in firmware/src/lcd.rs).
//! Every screen's text is made here, so the tests below pin down exactly what
//! the 16x2 display shows, and that nothing is ever cut off.

use core::fmt::Write;

use crate::control::PodState;
use crate::faults::{FaultKind, FaultLine};
use crate::layout::{cal_entry, row_plant, MAX_PLANTS, PLANTS_PER_ROW, ROWS, SENSORS};
use crate::learn::{Learn, LearnError, Phase};
use crate::manual::{manual_label, ManualItem};
use crate::plant::PlantCfg;
use crate::sensors::{ADC_MAX, ADC_VREF};

pub const LCD_COLS: usize = 16;

/// Small fixed text buffer for building an LCD line without an allocator.
pub struct Text {
    buf: [u8; LCD_COLS],
    len: usize,
}

impl Text {
    pub fn new() -> Self {
        Text { buf: [b' '; LCD_COLS], len: 0 }
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
    }
    /// Pad with spaces up to column `col`
    fn pad_to(&mut self, col: usize) {
        while self.len < col.min(LCD_COLS) {
            self.buf[self.len] = b' ';
            self.len += 1;
        }
    }
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
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

/// "50%", " 5%", "100" (3 characters)
fn pct3(t: &mut Text, pct: u8) {
    let _ = if pct >= 100 { write!(t, "100") } else { write!(t, "{:>2}%", pct) };
}

// ---------------------------------------------------------------------------
// Main screen: header, then one plant (up/down scrolls through the plants)
//   "R# P# ACT TGT SM"
//   "R1 P2 42% 50% OK"   ACT = moisture now, TGT = target, SM = state (PodState)
// ---------------------------------------------------------------------------
pub const MAIN_HEADER: &str = "R# P# ACT TGT SM";

/// `act` = None when the pod isn't installed. A 0 % target shows as OFF.
pub fn main_text(row: usize, plant: usize, act: Option<u8>, target: u8, state: PodState) -> Text {
    let mut t = Text::new();
    let _ = write!(t, "R{} P{} ", row + 1, plant + 1);
    match act {
        Some(pct) => pct3(&mut t, pct),
        None => {
            let _ = t.write_str("-- ");
        }
    }
    t.pad_to(10);
    if target == 0 {
        let _ = t.write_str("OFF");
    } else {
        pct3(&mut t, target);
    }
    t.pad_to(14);
    let _ = t.write_str(state.text());
    t
}

// ---------------------------------------------------------------------------
// PROFILE: pick a plant, then its water target and light hours
//   "PROFILE    R1 P2"   cursor on the plant: up/down picks the plant
//   "WTR 50%  LT 12H"    cursor on a value: up/down changes it (saved at once)
// ---------------------------------------------------------------------------
pub const PROFILE_PLANT_COL: u8 = 11;
pub const PROFILE_WATER_COL: u8 = 4;
pub const PROFILE_LIGHT_COL: u8 = 12;

pub fn profile_title(row: usize, plant: usize) -> Text {
    let mut t = Text::new();
    let _ = write!(t, "PROFILE    R{} P{}", row + 1, plant + 1);
    t
}

pub fn profile_values(cfg: &PlantCfg) -> Text {
    let mut t = Text::new();
    let _ = t.write_str("WTR ");
    if cfg.water == 0 {
        let _ = t.write_str("OFF");
    } else {
        pct3(&mut t, cfg.water);
    }
    let _ = t.write_str("  LT ");
    let _ = if cfg.light_h == 0 { t.write_str("OFF") } else { write!(t, "{:>2}H", cfg.light_h) };
    t
}

// ---------------------------------------------------------------------------
// TIME: two lines, the clock and the daily watering time
//   ">CLOCK     14:05"
//   " WATER     09:00"
// ---------------------------------------------------------------------------
pub fn time_line(selected: bool, label: &str, time: Option<(u8, u8)>) -> Text {
    let mut t = Text::new();
    let _ = t.write_str(if selected { ">" } else { " " });
    let _ = t.write_str(label);
    t.pad_to(11);
    let _ = match time {
        Some((h, m)) => write!(t, "{:02}:{:02}", h, m),
        None => t.write_str("--:--"),
    };
    t
}

// ---------------------------------------------------------------------------
// FAULTS: scrolling list, active faults first (most severe first)
//   "ACTIVE: 2"
//   "1 NO FLOW   R1P2"   severity, name, plant
//   "2 LOW WATER"
//   "INACTIVE: 8"
//   "1 LEAK        OK"
// ---------------------------------------------------------------------------
pub fn fault_text(kind: FaultKind, pod: Option<usize>) -> Text {
    let mut t = Text::new();
    let _ = write!(t, "{} {}", kind.severity(), kind.name());
    if let Some(i) = pod {
        let (r, p) = row_plant(i);
        t.pad_to(12);
        let _ = write!(t, "R{}P{}", r + 1, p + 1);
    }
    t
}

pub fn fault_line_text(line: FaultLine) -> Text {
    match line {
        FaultLine::Active(kind, pod) => fault_text(kind, pod),
        FaultLine::Inactive(kind) => {
            let mut t = fault_text(kind, None);
            t.pad_to(14);
            let _ = t.write_str("OK");
            t
        }
        FaultLine::ActiveHeader(n) => {
            let mut t = Text::new();
            let _ = write!(t, "ACTIVE: {}", n);
            t
        }
        FaultLine::InactiveHeader(n) => {
            let mut t = Text::new();
            let _ = write!(t, "INACTIVE: {}", n);
            t
        }
    }
}

// ---------------------------------------------------------------------------
// RELEARN, then pot learning for one plant
//   "LEARN POT: PICK"      up/down picks a plant, click starts
//   "R1P2  20mL/% DEF"     DEF = default (never learned), LRN = learned
//
//   "LEARN R1P2 DOSE1"     while running (hold button = abort)
//   " 30%> 33%  100mL"     start -> now moisture, water given
//
//   "R1P2 POT LEARNED"     result
//   "12 mL/% SAVED"
// ---------------------------------------------------------------------------
pub const LEARN_PICK_TITLE: &str = "LEARN POT: PICK";

pub fn learn_pick_text(row: usize, plant: usize, cfg: &PlantCfg) -> Text {
    let mut t = Text::new();
    let learned = cfg.ml_per_pct != crate::plant::POT_UNLEARNED;
    let _ = write!(
        t,
        "R{}P{} {:>3}mL/% {}",
        row + 1,
        plant + 1,
        cfg.pot_ml_per_pct(),
        if learned { "LRN" } else { "DEF" }
    );
    t
}

pub fn learn_title(learn: &Learn) -> Text {
    let (r, p) = row_plant(learn.idx);
    let mut t = Text::new();
    let _ = write!(t, "LEARN R{}P{} ", r + 1, p + 1);
    let _ = match learn.phase {
        Phase::Start => t.write_str("READ"),
        Phase::Pressure { .. } | Phase::Dose { .. } => write!(t, "DOSE{}", learn.doses.max(1)),
        Phase::Slowdown { .. } | Phase::Soak { .. } => t.write_str("SOAK"),
        Phase::Done(Ok(_)) => t.write_str("DONE"),
        Phase::Done(Err(_)) => t.write_str("FAIL"),
    };
    t
}

pub fn learn_status(learn: &Learn, pulses_now: u32) -> Text {
    let mut t = Text::new();
    if learn.phase == Phase::Start {
        let _ = t.write_str("READING SOIL...");
    } else {
        let _ = write!(t, "{:>3}%>{:>3}% {:>4}mL", learn.start_pct, learn.now_pct, learn.ml_given(pulses_now));
    }
    t
}

/// Two lines for the end of pot learning
pub fn learn_result(idx: usize, result: Result<u8, LearnError>) -> (Text, Text) {
    let (r, p) = row_plant(idx);
    let (mut a, mut b) = (Text::new(), Text::new());
    match result {
        Ok(ml) => {
            let _ = write!(a, "R{}P{} POT LEARNED", r + 1, p + 1);
            let _ = write!(b, "{} mL/% SAVED", ml);
        }
        Err(e) => {
            let _ = write!(a, "R{}P{} LEARN FAIL", r + 1, p + 1);
            let _ = b.write_str(e.text());
        }
    }
    (a, b)
}

// ---------------------------------------------------------------------------
// MANUAL: scrolling list of every input and output
//   ">JOY X     2048"     inputs: live raw value
//   " VLV R1P2  on OFF"   outputs: the state in capitals, click toggles
// ---------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ManualValue {
    Raw(u32),
    Closed(bool), // switch: CLOSED / OPEN
    On(bool),     // output state
}

pub fn manual_text(item: ManualItem, selected: bool, value: ManualValue) -> Text {
    let mut t = Text::new();
    let _ = t.write_str(if selected { ">" } else { " " });
    manual_label(item, &mut t);
    t.pad_to(10);
    let _ = match value {
        ManualValue::Raw(v) => write!(t, "{:>6}", v % 1_000_000),
        ManualValue::Closed(true) => t.write_str("CLOSED"),
        ManualValue::Closed(false) => t.write_str("  OPEN"),
        ManualValue::On(true) => t.write_str("ON off"),
        ManualValue::On(false) => t.write_str("on OFF"),
    };
    t
}

// ---------------------------------------------------------------------------
// CALIBRATE
// ---------------------------------------------------------------------------

/// One CALIBRATE line, e.g. "R1P1 1.1V LOW HI", or "R1P3 ---- --- --" if not detected.
/// `just_set` shows "SET!" in place of the voltage for a moment after a capture.
pub fn cal_text(i: usize, detected: &[bool; SENSORS], raw: &[u16; SENSORS], just_set: bool) -> Text {
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

/// Keep the selected line visible in a 2-line window (CALIBRATE, FAULTS, MANUAL).
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
    use crate::faults::{Faults, FAULT_KINDS};
    use crate::manual::{manual_item, MANUAL_COUNT};

    #[test]
    fn text_is_cut_at_16_chars() {
        let mut t = Text::new();
        let _ = write!(t, "0123456789ABCDEFGHIJ");
        assert_eq!(t.as_str(), "0123456789ABCDEF");
    }

    // ---- main screen ----
    #[test]
    fn main_line_examples() {
        assert_eq!(MAIN_HEADER.len(), 16);
        assert_eq!(main_text(0, 1, Some(42), 50, PodState::Ok).as_str(), "R1 P2 42% 50% OK");
        assert_eq!(main_text(1, 2, Some(100), 90, PodState::Watering).as_str(), "R2 P3 100 90% WT");
        assert_eq!(main_text(0, 0, Some(7), 0, PodState::Off).as_str(), "R1 P1  7% OFF OF");
        assert_eq!(main_text(1, 0, None, 50, PodState::Missing).as_str(), "R2 P1 --  50% --");
    }

    #[test]
    fn main_line_columns_sit_under_the_header() {
        let line = main_text(0, 1, Some(42), 50, PodState::Low);
        let l = line.as_str();
        for word in ["ACT", "TGT", "SM"] {
            let col = MAIN_HEADER.find(word).unwrap();
            assert_ne!(&l[col..col + 1], " ", "nothing under {word}: {l:?}");
        }
        assert_eq!(&l[14..], "LO");
    }

    // ---- profile ----
    #[test]
    fn profile_lines() {
        assert_eq!(profile_title(1, 2).as_str(), "PROFILE    R2 P3");
        let cfg = PlantCfg { water: 50, light_h: 12, ml_per_pct: 0 };
        assert_eq!(profile_values(&cfg).as_str(), "WTR 50%  LT 12H");
        let off = PlantCfg { water: 0, light_h: 0, ml_per_pct: 0 };
        assert_eq!(profile_values(&off).as_str(), "WTR OFF  LT OFF");
        let full = PlantCfg { water: 90, light_h: 24, ml_per_pct: 0 };
        assert_eq!(profile_values(&full).as_str(), "WTR 90%  LT 24H");
    }

    #[test]
    fn profile_cursors_sit_on_the_values() {
        let t = profile_title(0, 0);
        assert_eq!(&t.as_str()[PROFILE_PLANT_COL as usize..PROFILE_PLANT_COL as usize + 1], "R");
        let v = profile_values(&PlantCfg { water: 50, light_h: 8, ml_per_pct: 0 });
        assert_eq!(&v.as_str()[PROFILE_WATER_COL as usize..PROFILE_WATER_COL as usize + 3], "50%");
        assert_eq!(&v.as_str()[PROFILE_LIGHT_COL as usize..], " 8H");
    }

    // ---- time ----
    #[test]
    fn time_lines() {
        assert_eq!(time_line(true, "CLOCK", Some((14, 5))).as_str(), ">CLOCK     14:05");
        assert_eq!(time_line(false, "WATER", Some((9, 0))).as_str(), " WATER     09:00");
        assert_eq!(time_line(false, "CLOCK", None).as_str(), " CLOCK     --:--");
    }

    // ---- faults ----
    #[test]
    fn fault_lines() {
        assert_eq!(fault_text(FaultKind::NoFlow, Some(1)).as_str(), "1 NO FLOW   R1P2");
        assert_eq!(fault_text(FaultKind::LowWater, None).as_str(), "2 LOW WATER");
        assert_eq!(fault_line_text(FaultLine::Inactive(FaultKind::Leak)).as_str(), "1 LEAK        OK");
        assert_eq!(fault_line_text(FaultLine::ActiveHeader(2)).as_str(), "ACTIVE: 2");
    }

    #[test]
    fn every_fault_line_fits_with_its_plant() {
        for k in FAULT_KINDS {
            let t = fault_text(k, k.per_pod().then_some(crate::layout::PLANT_SLOTS - 1));
            assert!(t.as_str().len() <= 16, "{:?}", t.as_str());
            if k.per_pod() {
                assert!(t.as_str().ends_with("R2P3"), "{:?} lost its plant", t.as_str());
            }
        }
        let f = Faults::new();
        for i in 0..f.line_count() {
            assert!(!fault_line_text(f.line(i).unwrap()).as_str().is_empty());
        }
    }

    // ---- learning ----
    #[test]
    fn learn_lines() {
        let cfg = PlantCfg { water: 50, light_h: 12, ml_per_pct: 0 };
        assert_eq!(learn_pick_text(0, 1, &cfg).as_str(), "R1P2  20mL/% DEF");
        let learned = PlantCfg { ml_per_pct: 7, ..cfg };
        assert!(learn_pick_text(0, 1, &learned).as_str().ends_with("LRN"));
        let biggest = PlantCfg { ml_per_pct: 254, ..cfg };
        assert_eq!(learn_pick_text(1, 2, &biggest).as_str().len(), 16);
        assert!(LEARN_PICK_TITLE.len() <= 16);

        let mut l = Learn::new(1, 0);
        assert_eq!(learn_title(&l).as_str(), "LEARN R1P2 READ");
        assert_eq!(learn_status(&l, 0).as_str(), "READING SOIL...");
        l.start_pct = 30;
        l.now_pct = 33;
        l.doses = 1;
        l.phase = Phase::Soak { since: 0, last: None, steady: 0, next_read: 0 };
        assert_eq!(learn_title(&l).as_str(), "LEARN R1P2 SOAK");
        assert_eq!(learn_status(&l, 0).as_str(), " 30%> 33%    0mL");

        let (a, b) = learn_result(1, Ok(12));
        assert_eq!((a.as_str(), b.as_str()), ("R1P2 POT LEARNED", "12 mL/% SAVED"));
        let (a, b) = learn_result(5, Err(LearnError::NoRise));
        assert_eq!((a.as_str(), b.as_str()), ("R2P3 LEARN FAIL", "NO RISE IN SOIL"));
    }

    // ---- manual ----
    #[test]
    fn manual_lines() {
        assert_eq!(manual_text(ManualItem::JoyX, true, ManualValue::Raw(2048)).as_str(), ">JOY X      2048");
        assert_eq!(manual_text(ManualItem::Valve(1), false, ManualValue::On(false)).as_str(), " VLV R1P2 on OFF");
        assert_eq!(manual_text(ManualItem::Pump, true, ManualValue::On(true)).as_str(), ">PUMP     ON off");
        assert_eq!(manual_text(ManualItem::LevelSwitch, false, ManualValue::Closed(true)).as_str(), " LEVEL SW CLOSED");
    }

    #[test]
    fn every_manual_line_fits() {
        for i in 0..MANUAL_COUNT {
            let item = manual_item(i).unwrap();
            let value = if item.is_output() { ManualValue::On(true) } else { ManualValue::Raw(999_999) };
            assert_eq!(manual_text(item, true, value).as_str().len(), 16, "{item:?}");
        }
    }

    // ---- calibrate ----
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
