//! Per-plant user settings (set in MENU > PROFILE): water target, light hours,
//! and the pot size learned in MENU > RELEARN. Plus the daily light window.

use crate::params::*;

pub const SECONDS_PER_DAY: u32 = 86_400;

// Water setting = target soil moisture %, set in steps of 10%.
// 0% = OFF (that pod is never watered automatically).
// 90% is the max: 100% is the "fully wet" calibration point and can't be reached reliably.
pub const WATER_STEP: u8 = 10;
pub const WATER_MAX: u8 = 90;
pub const WATER_DEFAULT: u8 = 50;

// Light setting = hours of grow light per day, 0 (OFF) to 24, in 1 h steps.
pub const LIGHT_MAX_H: u8 = 24;

/// Pot size: mL of water that raise the soil reading by 1 %.
/// 0 = never learned: P_POT_ML_PER_PCT is used.
pub const POT_UNLEARNED: u8 = 0;

/// Everything the user sets for one plant.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PlantCfg {
    pub water: u8,      // target moisture %, 0 = OFF
    pub light_h: u8,    // grow light hours per day, 0 = OFF
    pub ml_per_pct: u8, // learned pot size, POT_UNLEARNED = use the default
}

impl PlantCfg {
    pub const DEFAULT: PlantCfg = PlantCfg { water: WATER_DEFAULT, light_h: P_LIGHT_DEFAULT_H, ml_per_pct: POT_UNLEARNED };

    /// Pot size used for dosing: learned, or the default for a large pot
    pub fn pot_ml_per_pct(&self) -> u8 {
        if self.ml_per_pct == POT_UNLEARNED {
            P_POT_ML_PER_PCT
        } else {
            self.ml_per_pct
        }
    }
}

/// Target one step up, stops at WATER_MAX
pub fn water_up(target: u8) -> u8 {
    (target + WATER_STEP).min(WATER_MAX)
}

/// Target one step down, stops at 0 (OFF)
pub fn water_down(target: u8) -> u8 {
    target.saturating_sub(WATER_STEP)
}

/// Light one hour more, stops at 24 h
pub fn light_up(hours: u8) -> u8 {
    (hours + 1).min(LIGHT_MAX_H)
}

/// Light one hour less, stops at 0 (OFF)
pub fn light_down(hours: u8) -> u8 {
    hours.saturating_sub(1)
}

/// Is `now` (seconds into the day) inside the daily window that starts at
/// `start` and lasts `len` seconds? Handles windows that cross midnight.
pub fn in_daily_window(now: u32, start: u32, len: u32) -> bool {
    (now + SECONDS_PER_DAY - start) % SECONDS_PER_DAY < len
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u32 = 3600;

    #[test]
    fn water_up_stops_at_max() {
        assert_eq!(water_up(80), 90);
        assert_eq!(water_up(WATER_MAX), WATER_MAX);
    }

    #[test]
    fn water_down_stops_at_off() {
        assert_eq!(water_down(10), 0);
        assert_eq!(water_down(0), 0);
    }

    #[test]
    fn light_steps_one_hour_between_off_and_24() {
        assert_eq!(light_up(12), 13);
        assert_eq!(light_up(LIGHT_MAX_H), LIGHT_MAX_H);
        assert_eq!(light_down(1), 0); // OFF
        assert_eq!(light_down(0), 0);
    }

    #[test]
    fn new_plant_starts_at_12_h_light_and_the_default_pot() {
        assert_eq!(PlantCfg::DEFAULT.light_h, 12);
        assert_eq!(PlantCfg::DEFAULT.pot_ml_per_pct(), P_POT_ML_PER_PCT);
    }

    #[test]
    fn learned_pot_size_replaces_the_default() {
        let cfg = PlantCfg { ml_per_pct: 7, ..PlantCfg::DEFAULT };
        assert_eq!(cfg.pot_ml_per_pct(), 7);
    }

    #[test]
    fn window_simple() {
        assert!(in_daily_window(8 * H, 7 * H, 10 * H));
        assert!(in_daily_window(7 * H, 7 * H, 10 * H)); // start is inclusive
        assert!(!in_daily_window(6 * H, 7 * H, 10 * H));
        assert!(!in_daily_window(17 * H, 7 * H, 10 * H)); // end is exclusive
    }

    #[test]
    fn window_crossing_midnight() {
        // lights on at 20:00 for 6 h -> off at 02:00
        assert!(in_daily_window(23 * H, 20 * H, 6 * H));
        assert!(in_daily_window(H, 20 * H, 6 * H));
        assert!(!in_daily_window(3 * H, 20 * H, 6 * H));
    }

    #[test]
    fn zero_hour_window_is_always_off() {
        for h in 0..24 {
            assert!(!in_daily_window(h * H, 7 * H, 0));
        }
    }

    #[test]
    fn full_day_window_is_always_on() {
        for h in 0..24 {
            assert!(in_daily_window(h * H, 7 * H, 24 * H));
        }
    }
}
