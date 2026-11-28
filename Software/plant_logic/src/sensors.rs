//! Hygrometers, reservoir level switch and sensor calibration.

use crate::layout::SENSORS;
use crate::params::*;

// Calibration: a sensor needs a LOW (dry) and HI (wet) reading before it can be
// used. Values set in the CALIBRATE menu are saved to flash and take priority;
// otherwise P_MOISTURE_DRY / P_MOISTURE_WET from params.dcm are used.
// Sensors with neither are treated as not installed.
pub const CAL_UNSET: u16 = 0xFFFF;
pub const CAL_MIN_SPAN: u16 = 100; // LOW and HI must differ by at least this many counts
pub const ADC_VREF: f32 = 3.3;
pub const ADC_MAX: f32 = 4095.0;

pub type Cal = [u16; SENSORS];

/// Is the reservoir low? `closed` = the level switch is closed (pin pulled to GND).
/// The switch is mounted closed = water OK, so open (or a broken wire) = low.
pub fn level_is_low(closed: bool) -> bool {
    !closed
}

/// Is a hygrometer actually plugged in on this channel?
///   Unplugged sensor: the 100k pull-down holds the pin near 0    -> below P_HYGRO_MIN_RAW
///   Faulty / out-of-range output (above the reading in air)       -> above P_HYGRO_MAX_RAW
/// Anything in between is a real soil reading.
/// (A capacitive sensor pulled out of the soil reads as very dry, not as unplugged.)
pub fn hygro_detected(raw: u16) -> bool {
    raw >= P_HYGRO_MIN_RAW && raw <= P_HYGRO_MAX_RAW
}

/// Calibration point for sensor `i`: the value saved from the menu if there is
/// one, else the params.dcm value, else CAL_UNSET.
pub fn cal_point(saved: &Cal, dcm: &[u16], i: usize) -> u16 {
    if saved[i] != CAL_UNSET {
        saved[i]
    } else if i < dcm.len() {
        dcm[i]
    } else {
        CAL_UNSET
    }
}

/// Effective (LOW/dry, HI/wet) calibration for sensor `i`, if usable.
pub fn cal_of(i: usize, cal_dry: &Cal, cal_wet: &Cal) -> Option<(u16, u16)> {
    let dry = cal_point(cal_dry, &P_MOISTURE_DRY, i);
    let wet = cal_point(cal_wet, &P_MOISTURE_WET, i);
    if dry != CAL_UNSET && wet != CAL_UNSET && dry.abs_diff(wet) >= CAL_MIN_SPAN {
        Some((dry, wet))
    } else {
        None
    }
}

/// Usable sensors: detected AND calibrated.
pub fn usable_sensors(detected: &[bool; SENSORS], cal_dry: &Cal, cal_wet: &Cal) -> [bool; SENSORS] {
    let mut ok = [false; SENSORS];
    for i in 0..SENSORS {
        ok[i] = detected[i] && cal_of(i, cal_dry, cal_wet).is_some();
    }
    ok
}

/// Raw ADC reading -> 0..100 % moisture, using that sensor's dry/wet calibration.
pub fn moisture_pct(raw: u16, dry: u16, wet: u16) -> u8 {
    let (raw, dry, wet) = (raw as i32, dry as i32, wet as i32);
    if dry == wet {
        return 0; // uncalibrated, avoid dividing by zero
    }
    let pct = (dry - raw) * 100 / (dry - wet);
    pct.clamp(0, 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- moisture_pct ----
    const DRY: u16 = 3000;
    const WET: u16 = 1200;

    #[test]
    fn dry_reading_is_0_pct() {
        assert_eq!(moisture_pct(DRY, DRY, WET), 0);
    }

    #[test]
    fn wet_reading_is_100_pct() {
        assert_eq!(moisture_pct(WET, DRY, WET), 100);
    }

    #[test]
    fn halfway_is_50_pct() {
        assert_eq!(moisture_pct(2100, DRY, WET), 50);
    }

    #[test]
    fn drier_than_dry_clamps_to_0() {
        assert_eq!(moisture_pct(4095, DRY, WET), 0);
    }

    #[test]
    fn wetter_than_wet_clamps_to_100() {
        assert_eq!(moisture_pct(0, DRY, WET), 100);
    }

    #[test]
    fn uncalibrated_does_not_divide_by_zero() {
        assert_eq!(moisture_pct(2000, 1500, 1500), 0);
    }

    // ---- detection ----
    #[test]
    fn hygro_detection_limits_are_inclusive() {
        assert!(hygro_detected(P_HYGRO_MIN_RAW));
        assert!(hygro_detected(P_HYGRO_MAX_RAW));
        assert!(!hygro_detected(P_HYGRO_MIN_RAW - 1)); // unplugged: pulled down to ~0
        assert!(!hygro_detected(P_HYGRO_MAX_RAW + 1)); // faulty
    }

    #[test]
    fn open_level_switch_means_low_water() {
        assert!(level_is_low(false)); // also what a broken wire looks like
        assert!(!level_is_low(true));
    }

    // ---- calibration ----
    #[test]
    fn menu_calibration_beats_params_dcm() {
        let mut dry = [CAL_UNSET; SENSORS];
        dry[0] = 2500;
        assert_eq!(cal_point(&dry, &P_MOISTURE_DRY, 0), 2500);
    }

    #[test]
    fn params_dcm_used_when_nothing_saved() {
        let unset = [CAL_UNSET; SENSORS];
        assert_eq!(cal_point(&unset, &P_MOISTURE_DRY, 0), P_MOISTURE_DRY[0]);
        assert_eq!(cal_point(&unset, &[], 0), CAL_UNSET); // no dcm value either
    }

    #[test]
    fn calibration_span_too_small_is_rejected() {
        let mut dry = [CAL_UNSET; SENSORS];
        let mut wet = [CAL_UNSET; SENSORS];
        dry[0] = 2000;
        wet[0] = 2000 - CAL_MIN_SPAN + 1;
        assert_eq!(cal_of(0, &dry, &wet), None);
        wet[0] = 2000 - CAL_MIN_SPAN;
        assert_eq!(cal_of(0, &dry, &wet), Some((2000, 2000 - CAL_MIN_SPAN)));
    }

    #[test]
    fn every_sensor_has_a_default_calibration() {
        // Without one, a pod only works after LOW *and* HI are both set in the
        // menu (row 2 used to be like this: params.dcm had only 3 values).
        let unset = [CAL_UNSET; SENSORS];
        for i in 0..SENSORS {
            assert!(cal_of(i, &unset, &unset).is_some(), "sensor {i} has no params.dcm calibration");
        }
    }

    #[test]
    fn undetected_sensor_is_never_usable() {
        let mut dry = [CAL_UNSET; SENSORS];
        let mut wet = [CAL_UNSET; SENSORS];
        dry[0] = 3000;
        wet[0] = 1200;
        let mut detected = [false; SENSORS];
        assert!(!usable_sensors(&detected, &dry, &wet)[0]);
        detected[0] = true;
        assert!(usable_sensors(&detected, &dry, &wet)[0]);
    }
}
