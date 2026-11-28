//! Joystick: centre correction and direction decoding.

use crate::params::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Dir {
    Center,
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    /// Code for the trace: 0 center, 1 up, 2 down, 3 left, 4 right
    pub fn code(self) -> u8 {
        match self {
            Dir::Center => 0,
            Dir::Up => 1,
            Dir::Down => 2,
            Dir::Left => 3,
            Dir::Right => 4,
        }
    }
}

// Joystick center learned by RELEARN: readings are shifted so the learned
// center becomes 2048 before the P_Joystick_LOW / HIGH thresholds are applied.
pub const JOY_IDEAL_CENTER: u16 = 2048;

/// Shift a raw axis reading so the learned center reads as 2048.
pub fn joy_correct(raw: u16, center: u16) -> u16 {
    (raw as i32 - center as i32 + JOY_IDEAL_CENTER as i32).clamp(0, 4095) as u16
}

pub fn joy_center_ok(center: u16) -> bool {
    center.abs_diff(JOY_IDEAL_CENTER) <= P_JOY_MAX_OFFSET
}

pub fn read_dir(x: u16, y: u16) -> Dir {
    // If a direction is reversed on your module, swap the matching pair here.
    if y < P_Joystick_LOW {
        Dir::Up
    } else if y > P_Joystick_HIGH {
        Dir::Down
    } else if x < P_Joystick_LOW {
        Dir::Left
    } else if x > P_Joystick_HIGH {
        Dir::Right
    } else {
        Dir::Center
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: u16 = JOY_IDEAL_CENTER;

    #[test]
    fn dir_codes_match_the_trace_docs() {
        let dirs = [Dir::Center, Dir::Up, Dir::Down, Dir::Left, Dir::Right];
        for (i, d) in dirs.iter().enumerate() {
            assert_eq!(d.code() as usize, i);
        }
    }

    #[test]
    fn resting_stick_is_center() {
        assert_eq!(read_dir(C, C), Dir::Center);
    }

    #[test]
    fn each_direction_is_decoded() {
        assert_eq!(read_dir(C, P_Joystick_LOW - 1), Dir::Up);
        assert_eq!(read_dir(C, P_Joystick_HIGH + 1), Dir::Down);
        assert_eq!(read_dir(P_Joystick_LOW - 1, C), Dir::Left);
        assert_eq!(read_dir(P_Joystick_HIGH + 1, C), Dir::Right);
    }

    #[test]
    fn thresholds_themselves_are_still_center() {
        assert_eq!(read_dir(P_Joystick_LOW, P_Joystick_HIGH), Dir::Center);
    }

    #[test]
    fn learned_center_maps_to_ideal_center() {
        assert_eq!(joy_correct(1900, 1900), C);
        assert_eq!(joy_correct(2200, 2200), C);
    }

    #[test]
    fn correction_never_leaves_adc_range() {
        assert_eq!(joy_correct(0, 3000), 0);
        assert_eq!(joy_correct(4095, 1000), 4095);
    }

    #[test]
    fn center_too_far_off_is_rejected() {
        assert!(joy_center_ok(C));
        assert!(joy_center_ok(C - P_JOY_MAX_OFFSET));
        // 0xFFFF = never learned (erased flash) must never count as a valid center
        assert!(!joy_center_ok(0xFFFF));
    }
}
