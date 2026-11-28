//! All saved settings, and how they turn into the payload bytes of a settings
//! record (record.rs) and back.
//!
//! Version 5 payload (54 bytes, 0xFF = not set):
//!   0..8    water target % per plant slot (up to 8 slots)
//!   8..16   light hours per day per plant slot (0 = OFF)
//!   16..24  learned pot size per plant slot, mL per % (0xFF / 0 = not learned)
//!   24..26  daily watering time: hour, minute
//!   26..38  calibration LOW (dry) per sensor, u16 little-endian (up to 6 sensors)
//!   38..50  calibration HI (wet) per sensor, u16 little-endian
//!   50..54  joystick center X, Y, u16 little-endian
//!
//! Older payloads (versions 2..4) were 16 plant cells followed by "extra" bytes
//! (water time, calibration for 8 sensors, joystick); `Settings::from_payload`
//! converts them, so an update keeps everything the user had set.

use crate::joystick::{joy_center_ok, JOY_IDEAL_CENTER};
use crate::layout::{row_plant, MAX_PLANTS, PLANT_SLOTS, ROWS, SENSORS};
use crate::params::*;
use crate::plant::{PlantCfg, LIGHT_MAX_H, POT_UNLEARNED, WATER_MAX, WATER_STEP};
use crate::record::PAYLOAD;
use crate::sensors::{Cal, CAL_UNSET};

/// [row][plant]; only the first PLANTS_PER_ROW[row] entries are used
pub type Plants = [[PlantCfg; MAX_PLANTS]; ROWS];

// Version 5 offsets
const WATER_AT: usize = 0;
const LIGHT_AT: usize = 8;
const POT_AT: usize = 16;
const TIME_AT: usize = 24;
const DRY_AT: usize = 26;
const WET_AT: usize = 38;
const JOY_AT: usize = 50;
const _: () = assert!(PLANT_SLOTS <= 8, "settings record holds up to 8 plants");
const _: () = assert!(SENSORS <= 6, "settings record holds calibration for up to 6 sensors");
const _: () = assert!(JOY_AT + 4 == PAYLOAD, "settings payload layout doesn't add up");

// Versions 2..4: 16 plant cells, then the extra bytes
const OLD_EXTRA_AT: usize = 16;
const OLD_DRY_AT: usize = OLD_EXTRA_AT + 2; // 8 sensors
const OLD_WET_AT: usize = OLD_EXTRA_AT + 18;
const OLD_JOY_AT: usize = OLD_EXTRA_AT + 34;
/// Hours of the old LOW / MED / HI light steps
const OLD_LIGHT_HOURS: [u8; 3] = [8, 12, 16];

pub const DEFAULT_WATER_TIME: (u8, u8) = (P_WATER_TIME_DEFAULT_H, P_WATER_TIME_DEFAULT_M);

/// Everything that is saved to flash
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Settings {
    pub plants: Plants,
    pub water_time: (u8, u8), // daily watering time (HH, MM)
    pub cal_dry: Cal,         // CALIBRATE menu values (CAL_UNSET = use params.dcm)
    pub cal_wet: Cal,
    pub joy_center: (u16, u16), // learned by RELEARN
}

impl Settings {
    pub const DEFAULT: Settings = Settings {
        plants: [[PlantCfg::DEFAULT; MAX_PLANTS]; ROWS],
        water_time: DEFAULT_WATER_TIME,
        cal_dry: [CAL_UNSET; SENSORS],
        cal_wet: [CAL_UNSET; SENSORS],
        joy_center: (JOY_IDEAL_CENTER, JOY_IDEAL_CENTER),
    };

    pub fn plant(&self, idx: usize) -> &PlantCfg {
        let (r, p) = row_plant(idx);
        &self.plants[r][p]
    }

    pub fn plant_mut(&mut self, idx: usize) -> &mut PlantCfg {
        let (r, p) = row_plant(idx);
        &mut self.plants[r][p]
    }

    pub fn to_payload(&self) -> [u8; PAYLOAD] {
        let mut b = [0xFF; PAYLOAD];
        for i in 0..PLANT_SLOTS {
            let cfg = self.plant(i);
            b[WATER_AT + i] = cfg.water;
            b[LIGHT_AT + i] = cfg.light_h;
            b[POT_AT + i] = cfg.ml_per_pct;
        }
        b[TIME_AT] = self.water_time.0;
        b[TIME_AT + 1] = self.water_time.1;
        for i in 0..SENSORS {
            put_u16(&mut b, DRY_AT + 2 * i, self.cal_dry[i]);
            put_u16(&mut b, WET_AT + 2 * i, self.cal_wet[i]);
        }
        put_u16(&mut b, JOY_AT, self.joy_center.0);
        put_u16(&mut b, JOY_AT + 2, self.joy_center.1);
        b
    }

    /// Settings from a saved payload. Anything not set, or out of range, keeps its default.
    pub fn from_payload(b: &[u8; PAYLOAD], version: u8) -> Settings {
        let mut s = Settings::DEFAULT;
        if version >= 5 {
            for i in 0..PLANT_SLOTS {
                let cfg = s.plant_mut(i);
                if b[WATER_AT + i] <= WATER_MAX && b[WATER_AT + i] % WATER_STEP == 0 {
                    cfg.water = b[WATER_AT + i];
                }
                if b[LIGHT_AT + i] <= LIGHT_MAX_H {
                    cfg.light_h = b[LIGHT_AT + i];
                }
                if b[POT_AT + i] != 0xFF {
                    cfg.ml_per_pct = b[POT_AT + i];
                }
            }
            s.water_time = time_or_default(b[TIME_AT], b[TIME_AT + 1]);
            for i in 0..SENSORS {
                s.cal_dry[i] = get_u16(b, DRY_AT + 2 * i);
                s.cal_wet[i] = get_u16(b, WET_AT + 2 * i);
            }
            s.joy_center = joy_or_default(get_u16(b, JOY_AT), get_u16(b, JOY_AT + 2));
        } else {
            for i in 0..PLANT_SLOTS {
                if let Some((water, light_h)) = old_cell(b[i], version) {
                    *s.plant_mut(i) = PlantCfg { water, light_h, ml_per_pct: POT_UNLEARNED };
                }
            }
            s.water_time = time_or_default(b[OLD_EXTRA_AT], b[OLD_EXTRA_AT + 1]);
            for i in 0..SENSORS {
                s.cal_dry[i] = get_u16(b, OLD_DRY_AT + 2 * i);
                s.cal_wet[i] = get_u16(b, OLD_WET_AT + 2 * i);
            }
            s.joy_center = joy_or_default(get_u16(b, OLD_JOY_AT), get_u16(b, OLD_JOY_AT + 2));
        }
        s
    }
}

fn put_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn get_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn time_or_default(hh: u8, mm: u8) -> (u8, u8) {
    if hh < 24 && mm < 60 {
        (hh, mm)
    } else {
        DEFAULT_WATER_TIME // 0xFF = never set
    }
}

fn joy_or_default(x: u16, y: u16) -> (u16, u16) {
    if joy_center_ok(x) && joy_center_ok(y) {
        (x, y)
    } else {
        (JOY_IDEAL_CENTER, JOY_IDEAL_CENTER) // 0xFFFF = never learned
    }
}

/// One plant cell of a version 2..4 record -> (water %, light hours), None if not set.
///   version 4:    bits 0-3 water / 10,       bits 4-5 light LOW/MED/HI
///   version 2, 3: bits 0-1 water LOW/MED/HI, bits 2-3 light LOW/MED/HI
fn old_cell(c: u8, version: u8) -> Option<(u8, u8)> {
    let (water, light) = if version >= 4 {
        let water = (c & 0x0F) * WATER_STEP;
        if water > WATER_MAX {
            return None;
        }
        (water, (c >> 4) & 0b11)
    } else {
        let level = (c & 0b11) as usize;
        if level > 2 {
            return None;
        }
        // LOW/MED/HI -> its P_TARGET_MOISTURE, rounded to the nearest 10 %
        let water = ((P_TARGET_MOISTURE[level] + WATER_STEP / 2) / WATER_STEP * WATER_STEP).min(WATER_MAX);
        (water, (c >> 2) & 0b11)
    };
    let light_h = *OLD_LIGHT_HOURS.get(light as usize)?;
    Some((water, light_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varied() -> Settings {
        let mut s = Settings::DEFAULT;
        for i in 0..PLANT_SLOTS {
            *s.plant_mut(i) = PlantCfg {
                water: ((i * 20) % 100) as u8 / WATER_STEP * WATER_STEP,
                light_h: (i * 5) as u8 % 25,
                ml_per_pct: if i % 2 == 0 { POT_UNLEARNED } else { 3 + i as u8 },
            };
        }
        s.water_time = (7, 30);
        for i in 0..SENSORS {
            s.cal_dry[i] = 3000 + i as u16;
            s.cal_wet[i] = 1200 + i as u16;
        }
        s.joy_center = (2000, 2100);
        s
    }

    #[test]
    fn settings_survive_a_save() {
        let s = varied();
        assert_eq!(Settings::from_payload(&s.to_payload(), 5), s);
    }

    #[test]
    fn every_water_and_light_setting_survives_a_save() {
        for water in (0..=WATER_MAX).step_by(WATER_STEP as usize) {
            for light_h in 0..=LIGHT_MAX_H {
                let mut s = Settings::DEFAULT;
                *s.plant_mut(PLANT_SLOTS - 1) = PlantCfg { water, light_h, ml_per_pct: 12 };
                assert_eq!(Settings::from_payload(&s.to_payload(), 5), s, "water {water} light {light_h}");
            }
        }
    }

    #[test]
    fn nothing_saved_gives_the_defaults() {
        for version in [2, 3, 4, 5] {
            assert_eq!(Settings::from_payload(&[0xFF; PAYLOAD], version), Settings::DEFAULT, "v{version}");
        }
    }

    #[test]
    fn default_water_time_is_9am() {
        assert_eq!(Settings::DEFAULT.water_time, (9, 0));
    }

    #[test]
    fn version_4_settings_convert() {
        let mut b = [0xFF; PAYLOAD];
        b[0] = 6 | (2 << 4); // 60 %, light HI
        b[1] = 0 | (0 << 4); // OFF, light LOW
        b[OLD_EXTRA_AT] = 7; // 07:15
        b[OLD_EXTRA_AT + 1] = 15;
        b[OLD_DRY_AT..OLD_DRY_AT + 2].copy_from_slice(&2900u16.to_le_bytes());
        b[OLD_JOY_AT..OLD_JOY_AT + 4].copy_from_slice(&[0xD0, 0x07, 0xD0, 0x07]); // 2000, 2000
        let s = Settings::from_payload(&b, 4);
        assert_eq!(*s.plant(0), PlantCfg { water: 60, light_h: 16, ml_per_pct: POT_UNLEARNED });
        assert_eq!(*s.plant(1), PlantCfg { water: 0, light_h: 8, ml_per_pct: POT_UNLEARNED });
        assert_eq!(*s.plant(2), PlantCfg::DEFAULT); // never set
        assert_eq!(s.water_time, (7, 15));
        assert_eq!(s.cal_dry[0], 2900);
        assert_eq!(s.cal_dry[1], CAL_UNSET);
        assert_eq!(s.joy_center, (2000, 2000));
    }

    #[test]
    fn version_3_settings_convert_to_valid_targets() {
        for level in 0..3u8 {
            let mut b = [0xFF; PAYLOAD];
            b[0] = level | (2 << 2); // water level, light HI
            let cfg = *Settings::from_payload(&b, 3).plant(0);
            assert_eq!(cfg.light_h, 16);
            assert_eq!(cfg.water % WATER_STEP, 0, "{} is not a 10 % step", cfg.water);
            assert!(cfg.water <= WATER_MAX);
            let wanted = P_TARGET_MOISTURE[level as usize];
            assert!(cfg.water.abs_diff(wanted.min(WATER_MAX)) <= WATER_STEP / 2, "{wanted} -> {}", cfg.water);
        }
    }

    #[test]
    fn bad_values_keep_the_defaults() {
        let mut b = Settings::DEFAULT.to_payload();
        b[WATER_AT] = 95; // not a 10 % step
        b[LIGHT_AT] = 25; // more than a day
        b[TIME_AT] = 24; // no such hour
        b[JOY_AT..JOY_AT + 2].copy_from_slice(&60_000u16.to_le_bytes()); // not a joystick reading
        assert_eq!(Settings::from_payload(&b, 5), Settings::DEFAULT);
    }
}
