//! Per-plant user settings: water target and light level, and the daily light window.

use crate::params::*;

pub const SECONDS_PER_DAY: u32 = 86_400;

// Water setting = target soil moisture %, set in steps of 10%.
// 0% = OFF (that pod is never watered automatically).
// 90% is the max: 100% is the "fully wet" calibration point and can't be reached reliably.
pub const WATER_STEP: u8 = 10;
pub const WATER_MAX: u8 = 90;
pub const WATER_DEFAULT: u8 = 50;

/// Target one step up, stops at WATER_MAX
pub fn water_up(target: u8) -> u8 {
    (target + WATER_STEP).min(WATER_MAX)
}

/// Target one step down, stops at 0 (OFF)
pub fn water_down(target: u8) -> u8 {
    target.saturating_sub(WATER_STEP)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LightLevel {
    Low,
    Med,
    High,
}

// Lt levels are hours of light per day: Low/Med/High = P_LIGHT_HOURS[0/1/2]
// (6 / 10 / 14 h). Shown on the LCD as hours, see light_hours().
impl LightLevel {
    pub fn up(self) -> Self {
        match self {
            LightLevel::Low => LightLevel::Med,
            LightLevel::Med | LightLevel::High => LightLevel::High,
        }
    }

    pub fn down(self) -> Self {
        match self {
            LightLevel::High => LightLevel::Med,
            LightLevel::Med | LightLevel::Low => LightLevel::Low,
        }
    }
}

/// Hours of light per day the user's Lt setting asks for.
pub fn light_hours(level: LightLevel) -> u32 {
    match level {
        LightLevel::Low => P_LIGHT_HOURS[0] as u32,
        LightLevel::Med => P_LIGHT_HOURS[1] as u32,
        LightLevel::High => P_LIGHT_HOURS[2] as u32,
    }
}

/// Is `now` (seconds into the day) inside the daily window that starts at
/// `start` and lasts `len` seconds? Handles windows that cross midnight.
pub fn in_daily_window(now: u32, start: u32, len: u32) -> bool {
    (now + SECONDS_PER_DAY - start) % SECONDS_PER_DAY < len
}

/// A preset for one type of plant. Picked in MENU > PROFILE, it sets the
/// selected plant's water target and light level in one go; both can still be
/// fine-tuned afterwards in the normal edit screen.
/// Water is the target soil moisture % (same scale as the edit screen), so
/// these are starting points: tune them once the sensors are calibrated.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Profile {
    pub name: &'static str, // up to PROFILE_NAME_MAX characters (LCD space)
    pub water: u8,
    pub light: LightLevel,
}

pub const PROFILE_NAME_MAX: usize = 8;

/// Add or change profiles here. `cargo test-logic` checks they fit the LCD,
/// use valid water steps and are all different.
pub const PROFILES: [Profile; 6] = [
    Profile { name: "HERBS", water: 50, light: LightLevel::High },    // basil, mint, parsley
    Profile { name: "GREENS", water: 70, light: LightLevel::High },   // lettuce, spinach
    Profile { name: "FLOWERS", water: 60, light: LightLevel::High },
    Profile { name: "TROPICAL", water: 60, light: LightLevel::Med },  // pothos, philodendron
    Profile { name: "FERN", water: 70, light: LightLevel::Low },
    Profile { name: "CACTUS", water: 20, light: LightLevel::High },   // and succulents
];

/// Index of the profile these settings match, if any (the PROFILE list starts there).
pub fn profile_of(water: u8, light: LightLevel) -> Option<usize> {
    PROFILES.iter().position(|p| p.water == water && p.light == light)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- profiles ----
    #[test]
    fn profile_names_fit_the_lcd() {
        for p in PROFILES {
            assert!(!p.name.is_empty() && p.name.len() <= PROFILE_NAME_MAX, "{:?}", p);
            assert!(p.name.bytes().all(|b| b.is_ascii_uppercase() || b == b' '), "{:?}", p);
        }
    }

    #[test]
    fn profile_water_is_a_valid_setting() {
        // Must be a value the edit screen can also reach, and not OFF
        for p in PROFILES {
            assert!(p.water > 0 && p.water <= WATER_MAX && p.water % WATER_STEP == 0, "{:?}", p);
        }
    }

    #[test]
    fn profiles_are_all_different() {
        for (i, a) in PROFILES.iter().enumerate() {
            for b in &PROFILES[i + 1..] {
                assert_ne!(a.name, b.name);
                assert!(a.water != b.water || a.light != b.light, "{} and {} are the same", a.name, b.name);
            }
        }
    }

    #[test]
    fn settings_find_their_profile() {
        for (i, p) in PROFILES.iter().enumerate() {
            assert_eq!(profile_of(p.water, p.light), Some(i));
        }
        assert_eq!(profile_of(0, LightLevel::Low), None); // OFF matches nothing
    }

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
    fn light_level_stops_at_the_ends() {
        assert_eq!(LightLevel::Low.down(), LightLevel::Low);
        assert_eq!(LightLevel::Low.up(), LightLevel::Med);
        assert_eq!(LightLevel::High.up(), LightLevel::High);
    }

    #[test]
    fn more_light_level_means_more_hours() {
        assert!(light_hours(LightLevel::Low) <= light_hours(LightLevel::Med));
        assert!(light_hours(LightLevel::Med) <= light_hours(LightLevel::High));
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
