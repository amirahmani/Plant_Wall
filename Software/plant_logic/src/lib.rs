//! Hardware-independent logic for the plant wall.
//!
//! Everything in this crate is plain computation: numbers in, numbers out.
//! It never touches pins, the ADC, I2C or flash, so it builds for the STM32
//! *and* for your PC. That is what makes it unit-testable:
//!
//!   cargo test-logic        (alias in .cargo/config.toml)
//!
//! The list of traced signals (what gets logged while running) is in signals.rs.
//!
//! The firmware crate (`firmware/`) does the hardware work and calls into here.
//! Rule of thumb when adding code: if a function doesn't need a peripheral,
//! put it here and write a test for it.

#![no_std]

// Tuning parameters generated from params.dcm by build.rs
#[allow(dead_code, non_upper_case_globals)]
pub mod params {
    include!(concat!(env!("OUT_DIR"), "/params.rs"));
}

pub mod control;
pub mod display;
pub mod joystick;
pub mod layout;
pub mod packing;
pub mod plant;
pub mod record;
pub mod sensors;
pub mod signals;
pub mod trace;

// Re-export everything so the firmware can simply `use plant_logic::*;`
pub use control::*;
pub use display::*;
pub use joystick::*;
pub use layout::*;
pub use packing::*;
pub use plant::*;
pub use record::{decode, encode, CELLS, EXTRA, RECORD_SIZE};
pub use sensors::*;
pub use signals::*;

use params::*;

// Catch params.dcm mistakes at compile time. Every sensor needs a default
// calibration, even if its probe isn't fitted yet: a sensor without one only
// becomes usable after both LOW and HI are set in the CALIBRATE menu.
const _: () = assert!(P_MOISTURE_DRY.len() == P_MOISTURE_WET.len(), "MOISTURE_DRY and MOISTURE_WET must have the same length");
const _: () = assert!(P_MOISTURE_DRY.len() == SENSORS, "params.dcm: P_MOISTURE_DRY/WET need one value per sensor (SENSORS in layout.rs)");
const _: () = assert!(P_MOISTURE_DRY.len() > 0, "MOISTURE calibration arrays must not be empty");
const _: () = assert!(P_MOISTURE_SAMPLES > 0, "MOISTURE_SAMPLES must be at least 1");
const _: () = assert!(P_TARGET_MOISTURE.len() == 3, "TARGET_MOISTURE needs LOW, MED, HI");
const _: () = assert!(P_MAX_CYCLES > 0, "MAX_CYCLES must be at least 1");
const _: () = assert!(P_DOSE_MAX_ML >= P_DOSE_MIN_ML, "DOSE_MAX_ML must be >= DOSE_MIN_ML");
const _: () = assert!(P_HOLD_SAMPLE_S > 0, "HOLD_SAMPLE_S must be above 0");
const _: () = assert!(P_HOLD_STEADY_COUNT > 0, "HOLD_STEADY_COUNT must be at least 1");
const _: () = assert!(P_HOLD_MAX_S >= P_HOLD_SAMPLE_S, "HOLD_MAX_S must be >= HOLD_SAMPLE_S");
const _: () = assert!(P_SYSCHK_STEP_MS > 0, "SYSCHK_STEP_MS must be above 0");
const _: () = assert!(P_LIGHT_HOURS.len() == 3, "LIGHT_HOURS needs LOW, MED, HI");
const _: () = assert!(
    P_LIGHT_HOURS[0] <= 24 && P_LIGHT_HOURS[1] <= 24 && P_LIGHT_HOURS[2] <= 24,
    "LIGHT_HOURS values must be 0..=24"
);
const _: () = assert!(P_LIGHT_START_H < 24, "LIGHT_START_H must be 0..=23");
