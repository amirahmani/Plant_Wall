//! Turning plant settings into the bytes of a settings record, and back.
//!
//! One byte per plant:
//!   bits 0-3 = water target / 10   (0..9 = 0..90 %)
//!   bits 4-5 = light               (0 = LOW, 1 = MED, 2 = HI step of P_LIGHT_HOURS)
//!   0xFF = nothing saved for that plant
//! Records older than settings version 4 used: bits 0-1 = water LOW/MED/HI,
//! bits 2-3 = light; those are converted on load (see unpack_v3).
//! Extra bytes: water time hour, minute, then LOW and HI calibration per sensor,
//! then the joystick center.

use crate::joystick::{joy_center_ok, JOY_IDEAL_CENTER};
use crate::layout::{MAX_PLANTS, ROWS, SENSORS};
use crate::params::*;
use crate::plant::{LightLevel, WATER_MAX, WATER_STEP};
use crate::record::{CELLS, EXTRA};
use crate::sensors::{Cal, CAL_UNSET};

/// [row][plant] = (water target %, light level)
pub type Plants = [[(u8, LightLevel); MAX_PLANTS]; ROWS];

pub const CAL_DRY_AT: usize = 2; // in the extra bytes, see record.rs
pub const CAL_WET_AT: usize = 18;
pub const JOY_X_AT: usize = 34;
pub const JOY_Y_AT: usize = 36;

const _: () = assert!(ROWS * MAX_PLANTS <= CELLS, "too many plants for the settings record");
const _: () = assert!(SENSORS <= 8, "settings record holds calibration for up to 8 sensors");
const _: () = assert!(CAL_WET_AT + SENSORS * 2 <= JOY_X_AT, "calibration overlaps the joystick bytes");
const _: () = assert!(JOY_Y_AT + 2 <= EXTRA, "settings record extra bytes too small");

pub fn light_code(light: LightLevel) -> u8 {
    match light {
        LightLevel::Low => 0,
        LightLevel::Med => 1,
        LightLevel::High => 2,
    }
}

pub fn light_from_code(code: u8) -> Option<LightLevel> {
    match code {
        0 => Some(LightLevel::Low),
        1 => Some(LightLevel::Med),
        2 => Some(LightLevel::High),
        _ => None,
    }
}

pub fn pack(plants: &Plants) -> [u8; CELLS] {
    let mut cells = [0xFF; CELLS];
    for r in 0..ROWS {
        for p in 0..MAX_PLANTS {
            let (water, light) = plants[r][p];
            cells[r * MAX_PLANTS + p] = (water / WATER_STEP) | (light_code(light) << 4);
        }
    }
    cells
}

/// Settings version 4 and later: water stored as target / 10.
pub fn unpack(cells: &[u8; CELLS], plants: &mut Plants) {
    for r in 0..ROWS {
        for p in 0..MAX_PLANTS {
            let c = cells[r * MAX_PLANTS + p];
            let water = (c & 0x0F) * WATER_STEP;
            if water > WATER_MAX {
                continue; // 0xFF or invalid: keep the default
            }
            let Some(light) = light_from_code((c >> 4) & 0b11) else {
                continue;
            };
            plants[r][p] = (water, light);
        }
    }
}

/// Older records (before version 4): water was LOW/MED/HI. Convert each to the
/// matching P_TARGET_MOISTURE value, rounded to the nearest 10 %.
pub fn unpack_v3(cells: &[u8; CELLS], plants: &mut Plants) {
    for r in 0..ROWS {
        for p in 0..MAX_PLANTS {
            let c = cells[r * MAX_PLANTS + p];
            let level = (c & 0b11) as usize;
            if level > 2 {
                continue; // 0xFF or invalid: keep the default
            }
            let Some(light) = light_from_code((c >> 2) & 0b11) else {
                continue;
            };
            let water = ((P_TARGET_MOISTURE[level] + WATER_STEP / 2) / WATER_STEP * WATER_STEP).min(WATER_MAX);
            plants[r][p] = (water, light);
        }
    }
}

/// Extra bytes of a record: water time, menu calibration and joystick center.
pub fn pack_extra(water_time: (u8, u8), cal_dry: &Cal, cal_wet: &Cal, joy_center: (u16, u16)) -> [u8; EXTRA] {
    let mut extra = [0xFF; EXTRA];
    extra[0] = water_time.0;
    extra[1] = water_time.1;
    for i in 0..SENSORS {
        extra[CAL_DRY_AT + 2 * i..CAL_DRY_AT + 2 * i + 2].copy_from_slice(&cal_dry[i].to_le_bytes());
        extra[CAL_WET_AT + 2 * i..CAL_WET_AT + 2 * i + 2].copy_from_slice(&cal_wet[i].to_le_bytes());
    }
    extra[JOY_X_AT..JOY_X_AT + 2].copy_from_slice(&joy_center.0.to_le_bytes());
    extra[JOY_Y_AT..JOY_Y_AT + 2].copy_from_slice(&joy_center.1.to_le_bytes());
    extra
}

/// Joystick center saved by RELEARN, or the ideal center if never learned / invalid.
pub fn unpack_joy(extra: &[u8; EXTRA]) -> (u16, u16) {
    let x = u16::from_le_bytes([extra[JOY_X_AT], extra[JOY_X_AT + 1]]);
    let y = u16::from_le_bytes([extra[JOY_Y_AT], extra[JOY_Y_AT + 1]]);
    if joy_center_ok(x) && joy_center_ok(y) {
        (x, y)
    } else {
        (JOY_IDEAL_CENTER, JOY_IDEAL_CENTER) // 0xFFFF = never learned
    }
}

/// Calibration saved from the menu, out of the extra bytes (CAL_UNSET if never set).
pub fn unpack_cal(extra: &[u8; EXTRA]) -> (Cal, Cal) {
    let mut dry = [CAL_UNSET; SENSORS];
    let mut wet = [CAL_UNSET; SENSORS];
    for i in 0..SENSORS {
        dry[i] = u16::from_le_bytes([extra[CAL_DRY_AT + 2 * i], extra[CAL_DRY_AT + 2 * i + 1]]);
        wet[i] = u16::from_le_bytes([extra[CAL_WET_AT + 2 * i], extra[CAL_WET_AT + 2 * i + 1]]);
    }
    (dry, wet)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: (u8, LightLevel) = (50, LightLevel::Med);

    fn varied_plants() -> Plants {
        let lights = [LightLevel::Low, LightLevel::Med, LightLevel::High];
        let mut plants = [[DEFAULT; MAX_PLANTS]; ROWS];
        for r in 0..ROWS {
            for p in 0..MAX_PLANTS {
                let i = r * MAX_PLANTS + p;
                plants[r][p] = (((i * 20) % 100) as u8, lights[i % 3]);
            }
        }
        plants
    }

    #[test]
    fn plants_pack_then_unpack_round_trips() {
        let saved = varied_plants();
        let mut loaded = [[DEFAULT; MAX_PLANTS]; ROWS];
        unpack(&pack(&saved), &mut loaded);
        assert_eq!(loaded, saved);
    }

    #[test]
    fn every_water_and_light_setting_survives_a_save() {
        for water in (0..=WATER_MAX).step_by(WATER_STEP as usize) {
            for light in [LightLevel::Low, LightLevel::Med, LightLevel::High] {
                let saved = [[(water, light); MAX_PLANTS]; ROWS];
                let mut loaded = [[DEFAULT; MAX_PLANTS]; ROWS];
                unpack(&pack(&saved), &mut loaded);
                assert_eq!(loaded, saved, "water {water} light {light:?}");
            }
        }
    }

    #[test]
    fn nothing_saved_keeps_the_defaults() {
        let mut plants = [[DEFAULT; MAX_PLANTS]; ROWS];
        unpack(&[0xFF; CELLS], &mut plants);
        assert_eq!(plants, [[DEFAULT; MAX_PLANTS]; ROWS]);
        unpack_v3(&[0xFF; CELLS], &mut plants);
        assert_eq!(plants, [[DEFAULT; MAX_PLANTS]; ROWS]);
    }

    #[test]
    fn old_v3_settings_convert_to_valid_targets() {
        for level in 0..3u8 {
            let mut cells = [0xFF; CELLS];
            cells[0] = level | (2 << 2); // water level, light HI
            let mut plants = [[DEFAULT; MAX_PLANTS]; ROWS];
            unpack_v3(&cells, &mut plants);
            let (water, light) = plants[0][0];
            assert_eq!(light, LightLevel::High);
            assert_eq!(water % WATER_STEP, 0, "{water} is not a 10 % step");
            assert!(water <= WATER_MAX);
            let wanted = P_TARGET_MOISTURE[level as usize];
            assert!(water.abs_diff(wanted.min(WATER_MAX)) <= WATER_STEP / 2, "{wanted} -> {water}");
        }
    }

    #[test]
    fn extra_bytes_round_trip() {
        let mut dry = [CAL_UNSET; SENSORS];
        let mut wet = [CAL_UNSET; SENSORS];
        for i in 0..SENSORS {
            dry[i] = 3000 + i as u16;
            wet[i] = 1200 + i as u16;
        }
        let extra = pack_extra((7, 30), &dry, &wet, (2000, 2100));
        assert_eq!((extra[0], extra[1]), (7, 30));
        assert_eq!(unpack_cal(&extra), (dry, wet));
        assert_eq!(unpack_joy(&extra), (2000, 2100));
    }

    #[test]
    fn never_learned_joystick_uses_ideal_center() {
        assert_eq!(unpack_joy(&[0xFF; EXTRA]), (JOY_IDEAL_CENTER, JOY_IDEAL_CENTER));
    }

    #[test]
    fn never_calibrated_reads_as_unset() {
        assert_eq!(unpack_cal(&[0xFF; EXTRA]), ([CAL_UNSET; SENSORS], [CAL_UNSET; SENSORS]));
    }
}
