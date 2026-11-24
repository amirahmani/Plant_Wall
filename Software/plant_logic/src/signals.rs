//! SIGNAL LIST: every signal that is traced while the system runs.
//!
//! This is the measurement counterpart to params.dcm (like an A2L file): each
//! signal is declared once here, with its name, type and unit, in the group
//! for its sample rate. The firmware fills these structs and sends them over
//! the ST-LINK USB serial port; `tools/trace_record.py` saves them as CSV.
//!
//! # To add a signal
//! 1. Add a line to the group with the right sample rate, e.g.
//!    `tank_temp: u16, "0.1C";`
//!    Types: u8, u16, u32, u64, bool, or arrays of those (one column each).
//!    Use integers in a unit that fits (mL instead of L, mL/min instead of L/min).
//! 2. `cargo build` will fail until you fill it in where the group is built in
//!    firmware/src/main.rs (search for the struct name). That's on purpose:
//!    a signal can't be silently left out.
//! 3. Nothing to change on the PC: the recorder reads the header.

use crate::layout::{LIGHTS, PLANT_SLOTS, SENSORS, VALVES};
use crate::trace::signal_group;

signal_group! {
    /// Fast inputs, every 10 ms tick.
    ///
    /// `joy_dir`: 0 center, 1 up, 2 down, 3 left, 4 right
    pub struct FastTrace, tag = b'F', period_ms = 10 {
        joy_x: u16, "raw";            // joystick X, raw ADC (before center correction)
        joy_y: u16, "raw";            // joystick Y, raw ADC
        joy_dir: u8, "enum";          // decoded direction (after center correction)
        button: bool, "bool";         // joystick button held
        low_water_raw: bool, "bool";  // level switch says low right now (before debounce)
        flow_pulses: u32, "pulses";   // flow meter pulses since power-on
    }
}

signal_group! {
    /// Control loop, outputs and sensors, every 100 ms (after the watering step).
    ///
    /// `mode`:        0 browse, 1 edit, 2 menu, 3 relearn, 4 relearn result,
    ///                5 sys check, 6 sys result, 7 calibrate, 8 set time
    /// `water_state`: 0 standby, 1 evaluate, 2 pre-control, 3 flow active, 4 slowdown, 5 hold
    /// `fault`:       0 none, 1 no flow, 2 not reached
    pub struct ControlTrace, tag = b'C', period_ms = 100 {
        mode: u8, "enum";                  // HMI screen
        water_state: u8, "enum";           // watering state machine
        water_pod: u8, "idx";              // plant slot being watered (255 = none)
        water_cycle: u8, "count";          // doses given to that pod in this run
        dose_ml: u16, "mL";                // last PID output
        pid_i_ml: u16, "mL";               // PID integral term
        goal_pulses: u32, "pulses";        // dose of the current pump run, in flow pulses
        delivered_pulses: u32, "pulses";   // delivered so far (FLOW ACTIVE only)
        pump: bool, "bool";                // pump output
        valve: [bool; VALVES], "bool";     // valve outputs
        led: bool, "bool";                 // case LED output
        low_water: bool, "bool";           // debounced LOW WATER warning
        fault: u8, "enum";                 // last watering fault
        fault_pod: u8, "idx";              // plant slot of that fault (255 = none)
        save_failed: bool, "bool";         // last flash save failed
        overruns: u32, "count";            // 10 ms ticks that ran too long, since power-on
        moisture_reads: u32, "count";      // completed hygrometer readings
        raw: [u16; SENSORS], "raw";        // last hygrometer reading, raw ADC
        moist: [u8; SENSORS], "%";         // moisture
        detected: [bool; SENSORS], "bool"; // sensor plugged in
        usable: [bool; SENSORS], "bool";   // detected AND calibrated
        log_dropped: u32, "lines";         // trace lines lost because the queue was full
    }
}

signal_group! {
    /// Slow values, every 1 s (after the 1 s task).
    pub struct SlowTrace, tag = b'S', period_ms = 1000 {
        uptime_s: u32, "s";                // seconds since power-on
        tod_s: u32, "s";                   // time of day (only valid when clock_set = 1)
        clock_set: bool, "bool";
        flow: u32, "mL/min";               // flow over the last second
        total_ml: u32, "mL";               // water pumped since power-on
        light: [bool; LIGHTS], "bool";     // grow light outputs
        target: [u8; PLANT_SLOTS], "%";    // user water target per plant slot (0 = OFF)
        light_h: [u8; PLANT_SLOTS], "h";   // user light hours per plant slot
        water_time_min: u16, "min";        // daily water time, minutes after midnight
        water_run_due: bool, "bool";       // a watering run is waiting to start
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::trace::{Line, LINE_MAX};
    use std::format;

    fn text<const N: usize>(line: &Line<N>) -> &str {
        core::str::from_utf8(line.as_bytes()).unwrap()
    }

    /// Number of comma-separated fields in a line
    fn fields(s: &str) -> usize {
        s.trim_end().split(',').count()
    }

    #[test]
    fn fast_frame_looks_right() {
        let f = FastTrace { joy_x: 2051, joy_y: 7, joy_dir: 3, button: true, low_water_raw: false, flow_pulses: 12345 };
        let mut l = Line::<LINE_MAX>::new();
        f.frame(1230, &mut l);
        assert_eq!(text(&l), "F,1230,2051,7,3,1,0,12345\n");
    }

    #[test]
    fn fast_header_looks_right() {
        let mut l = Line::<LINE_MAX>::new();
        FastTrace::header(&mut l);
        assert_eq!(
            text(&l),
            "#F,10,t_ms[ms],joy_x[raw],joy_y[raw],joy_dir[enum],button[bool],low_water_raw[bool],flow_pulses[pulses]\n"
        );
    }

    #[test]
    fn arrays_become_numbered_columns() {
        let mut l = Line::<LINE_MAX>::new();
        ControlTrace::header(&mut l);
        let h = text(&l);
        assert!(h.contains(",moist0[%],moist1[%],"), "{h}");
        assert!(h.contains(&format!(",moist{}[%]", SENSORS - 1)), "{h}");
        assert!(!h.contains(&format!(",moist{}[", SENSORS)), "{h}");
    }

    /// The most important test: header and data always have the same columns
    fn check_group(header: impl Fn(&mut Line<LINE_MAX>), frame: impl Fn(&mut Line<LINE_MAX>), tag: u8) {
        let mut h = Line::<LINE_MAX>::new();
        let mut f = Line::<LINE_MAX>::new();
        header(&mut h);
        frame(&mut f);
        assert!(h.ok() && f.ok(), "line too long for LINE_MAX");
        let (h, f) = (text(&h), text(&f));
        // header has "#X,period" before t_ms, data only "X"
        assert_eq!(fields(h) - 1, fields(f), "\nheader: {h}\nframe:  {f}");
        assert_eq!(h.as_bytes()[1], tag);
        assert_eq!(f.as_bytes()[0], tag);
    }

    #[test]
    fn every_group_header_matches_its_data() {
        check_group(FastTrace::header, |l| FastTrace::default().frame(0, l), FastTrace::TAG);
        check_group(ControlTrace::header, |l| ControlTrace::default().frame(0, l), ControlTrace::TAG);
        check_group(SlowTrace::header, |l| SlowTrace::default().frame(0, l), SlowTrace::TAG);
    }

    #[test]
    fn worst_case_lines_fit() {
        assert!(FastTrace::MAX_LINE <= LINE_MAX, "{}", FastTrace::MAX_LINE);
        assert!(ControlTrace::MAX_LINE <= LINE_MAX, "{}", ControlTrace::MAX_LINE);
        assert!(SlowTrace::MAX_LINE <= LINE_MAX, "{}", SlowTrace::MAX_LINE);
    }

    #[test]
    fn largest_values_produce_max_line() {
        let f = ControlTrace {
            mode: u8::MAX,
            water_state: u8::MAX,
            water_pod: u8::MAX,
            water_cycle: u8::MAX,
            dose_ml: u16::MAX,
            pid_i_ml: u16::MAX,
            goal_pulses: u32::MAX,
            delivered_pulses: u32::MAX,
            fault: u8::MAX,
            fault_pod: u8::MAX,
            overruns: u32::MAX,
            moisture_reads: u32::MAX,
            raw: [u16::MAX; SENSORS],
            moist: [u8::MAX; SENSORS],
            log_dropped: u32::MAX,
            ..Default::default()
        };
        let mut l = Line::<LINE_MAX>::new();
        f.frame(u64::MAX, &mut l);
        assert!(l.ok());
        assert_eq!(l.as_bytes().len(), ControlTrace::MAX_LINE);
    }

    #[test]
    fn names_and_units_are_csv_safe() {
        for s in FastTrace::SIGNALS.iter().chain(ControlTrace::SIGNALS).chain(SlowTrace::SIGNALS) {
            for c in s.name.chars().chain(s.unit.chars()) {
                assert!(!matches!(c, ',' | '[' | ']' | '\n' | '"' | '#'), "bad char {c:?} in {}", s.name);
            }
        }
    }

    #[test]
    fn tags_are_unique() {
        let tags = [FastTrace::TAG, ControlTrace::TAG, SlowTrace::TAG];
        assert!(tags[0] != tags[1] && tags[1] != tags[2] && tags[0] != tags[2]);
        assert!(!tags.contains(&b'#'));
    }

    #[test]
    fn serial_bandwidth_has_headroom() {
        // Worst case bytes per second if every group sends its longest line,
        // must stay under half of what 230400 baud carries (10 bits per byte).
        let per_s = FastTrace::MAX_LINE * (1000 / FastTrace::PERIOD_MS as usize)
            + ControlTrace::MAX_LINE * (1000 / ControlTrace::PERIOD_MS as usize)
            + SlowTrace::MAX_LINE * (1000 / SlowTrace::PERIOD_MS as usize);
        let capacity = 230_400 / 10;
        assert!(per_s * 2 <= capacity, "{per_s} B/s worst case, capacity {capacity} B/s");
    }
}
