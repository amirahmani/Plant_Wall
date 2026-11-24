//! Control loop for each plant: state machine + PID dosing.
//!
//! Once a day at the WATER TIME (or every P_WATER_INTERVAL_S while the clock
//! isn't set), go through the installed pods one at a time (in series).
//!
//! ```text
//!   STANDBY      nothing running, waiting for the next watering run
//!      |  run due
//!      v
//!   EVALUATE     (one 100 ms step) PID: target vs current moisture -> dose in mL
//!      |  dose needed                  | at target / OFF -> next pod
//!      v                               | max cycles used -> NOT WET fault, next pod
//!   ACTIVE
//!     PRE-CONTROL  pump on, valve closed, P_PRECONTROL_MS to build pressure
//!     FLOW ACTIVE  valve open until the flow meter counts the dose
//!                  (also stops on P_PUMP_MAX_S, no flow -> NO FLOW fault, or pod unplugged)
//!      v
//!   SLOWDOWN     valve closes, P_SLOWDOWN_MS later the pump stops
//!      v
//!   HOLD         read moisture every P_HOLD_SAMPLE_S until it is steady
//!                (changes <= P_HOLD_STEADY_PCT for P_HOLD_STEADY_COUNT readings
//!                in a row) or P_HOLD_MAX_S passes, then back to EVALUATE
//! ```
//!
//! PID (evaluated once per cycle, output = dose in mL):
//!
//! ```text
//!   error = target % - moisture %
//!   P = P_PID_KP * error
//!   I = sum of P_PID_KI * error over this pod's cycles, kept in 0..=P_PID_I_MAX (anti-windup)
//!   D = -P_PID_KD * (moisture change since the last cycle)   (on measurement: no kick)
//!   dose = P + I + D, clamped to P_DOSE_MIN_ML..=P_DOSE_MAX_ML
//! ```
//!
//! The pod counts as done once error <= P_DEADBAND_PCT. Water can only be added,
//! so the integral never goes negative. The PID resets for each pod.
//!
//! The state machine is stepped every 100 ms (in the firmware), so the display,
//! joystick and flow meter keep running the whole time.

use crate::layout::{sensor_valve, PLANT_SLOTS, SENSORS};
use crate::params::*;

/// ACTIVE substates
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ActiveSub {
    PreControl, // pump on, valve closed: building pressure
    FlowActive, // valve open: delivering the dose
}

/// Why a pump run ended (decides what happens after SLOWDOWN)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RunEnd {
    Done,    // dose delivered, or timed out: go to HOLD
    NoFlow,  // pump ran but the flow meter saw nothing: fault, next pod
    Removed, // pod unplugged mid-run: next pod
}

/// Watering state machine. `idx` = plant slot, `cycle` = doses given to this pod so far.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Water {
    Standby,
    Evaluate { idx: usize, cycle: u8 },
    Active { idx: usize, cycle: u8, sub: ActiveSub, start_n: u32, start_s: u32, start_pulses: u32, goal: u32 },
    Slowdown { idx: usize, cycle: u8, start_n: u32, end: RunEnd },
    Hold { idx: usize, cycle: u8, start_s: u32, reads: u32, last: Option<u8>, steady: u8, next_sample_s: u32 },
}

impl Water {
    /// Code for the trace: 0 standby, 1 evaluate, 2 pre-control, 3 flow active, 4 slowdown, 5 hold
    pub fn code(&self) -> u8 {
        match self {
            Water::Standby => 0,
            Water::Evaluate { .. } => 1,
            Water::Active { sub: ActiveSub::PreControl, .. } => 2,
            Water::Active { sub: ActiveSub::FlowActive, .. } => 3,
            Water::Slowdown { .. } => 4,
            Water::Hold { .. } => 5,
        }
    }

    /// Plant slot being worked on, if any
    pub fn pod(&self) -> Option<usize> {
        match *self {
            Water::Standby => None,
            Water::Evaluate { idx, .. }
            | Water::Active { idx, .. }
            | Water::Slowdown { idx, .. }
            | Water::Hold { idx, .. } => Some(idx),
        }
    }

    /// Doses given to that pod so far (0 in standby)
    pub fn cycle(&self) -> u8 {
        match *self {
            Water::Standby => 0,
            Water::Evaluate { cycle, .. }
            | Water::Active { cycle, .. }
            | Water::Slowdown { cycle, .. }
            | Water::Hold { cycle, .. } => cycle,
        }
    }
}

/// PID state for the pod being watered (reset for each pod)
#[derive(Clone, Copy, Debug)]
pub struct Pid {
    integral: f32,          // mL, already scaled by Ki
    last_meas: Option<f32>, // moisture % at the previous cycle, for D
}

impl Pid {
    pub const fn new() -> Self {
        Pid { integral: 0.0, last_meas: None }
    }

    /// Integral term in mL (for the trace)
    pub fn integral(&self) -> f32 {
        self.integral
    }

    /// One PID step: returns the dose in mL.
    pub fn update(&mut self, error: f32, meas: f32) -> f32 {
        self.integral = (self.integral + P_PID_KI * error).clamp(0.0, P_PID_I_MAX);
        let d = match self.last_meas {
            Some(prev) => -P_PID_KD * (meas - prev),
            None => 0.0, // first cycle: no history yet
        };
        self.last_meas = Some(meas);
        (P_PID_KP * error + self.integral + d).clamp(P_DOSE_MIN_ML, P_DOSE_MAX_ML)
    }
}

/// What EVALUATE decided for a pod
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Decision {
    Dose(f32),  // water this many mL
    Done,       // at target (or OFF)
    NotReached, // max cycles used and still below target
}

pub fn decide(target: u8, current: u8, cycle: u8, pid: &mut Pid) -> Decision {
    if target == 0 {
        return Decision::Done; // OFF: never watered automatically
    }
    let error = target as f32 - current as f32;
    if error <= P_DEADBAND_PCT as f32 {
        return Decision::Done;
    }
    if cycle >= P_MAX_CYCLES {
        return Decision::NotReached;
    }
    Decision::Dose(pid.update(error, current as f32))
}

/// mL -> flow meter pulses (at least 1)
pub fn ml_to_pulses(ml: f32) -> u32 {
    ((ml * P_FLOW_PULSES_PER_L / 1000.0) as u32).max(1)
}

/// flow meter pulses -> mL
pub fn pulses_to_ml(pulses: u32) -> u32 {
    (pulses as f32 * 1000.0 / P_FLOW_PULSES_PER_L) as u32
}

/// Move on to the first installed pod at or after `from` (fresh PID), or STANDBY
/// when the run is done.
pub fn next_pod(from: usize, hygro_ok: &[bool; SENSORS], pid: &mut Pid) -> Water {
    match (from..PLANT_SLOTS).find(|&i| sensor_valve(i, hygro_ok).is_some()) {
        Some(idx) => {
            *pid = Pid::new();
            Water::Evaluate { idx, cycle: 0 }
        }
        None => Water::Standby,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fault {
    NoFlow(usize),     // pump ran but the flow meter saw nothing
    NotReached(usize), // max cycles used and still below target
}

impl Fault {
    /// Code for the trace: 1 no flow, 2 not reached (0 = no fault, see `fault_code`)
    pub fn code(&self) -> u8 {
        match self {
            Fault::NoFlow(_) => 1,
            Fault::NotReached(_) => 2,
        }
    }

    pub fn pod(&self) -> usize {
        match *self {
            Fault::NoFlow(idx) | Fault::NotReached(idx) => idx,
        }
    }
}

/// Trace code of an optional fault: 0 = none
pub fn fault_code(fault: Option<Fault>) -> u8 {
    fault.map_or(0, |f| f.code())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- decide ----
    #[test]
    fn off_pod_is_never_watered() {
        assert_eq!(decide(0, 10, 0, &mut Pid::new()), Decision::Done);
    }

    #[test]
    fn inside_deadband_is_done() {
        assert_eq!(decide(50, 50 - P_DEADBAND_PCT, 0, &mut Pid::new()), Decision::Done);
    }

    #[test]
    fn wetter_than_target_is_done() {
        assert_eq!(decide(50, 80, 0, &mut Pid::new()), Decision::Done);
    }

    #[test]
    fn just_outside_deadband_gets_water() {
        let d = decide(50, 50 - P_DEADBAND_PCT - 1, 0, &mut Pid::new());
        assert!(matches!(d, Decision::Dose(_)), "got {d:?}");
    }

    #[test]
    fn too_many_cycles_is_a_fault() {
        assert_eq!(decide(50, 10, P_MAX_CYCLES, &mut Pid::new()), Decision::NotReached);
    }

    // ---- PID ----
    #[test]
    fn dose_is_always_within_limits() {
        for current in 0..=90u8 {
            if let Decision::Dose(ml) = decide(90, current, 0, &mut Pid::new()) {
                assert!(ml >= P_DOSE_MIN_ML && ml <= P_DOSE_MAX_ML, "dose {ml} at {current}%");
            }
        }
    }

    #[test]
    fn dose_stays_within_limits_over_many_cycles() {
        // Even if the soil never gets wetter, the integral must not wind up past the limits
        let mut pid = Pid::new();
        for _ in 0..1000 {
            let ml = pid.update(90.0, 0.0);
            assert!(ml >= P_DOSE_MIN_ML && ml <= P_DOSE_MAX_ML, "dose {ml}");
        }
    }

    #[test]
    fn repeated_shortfall_never_lowers_the_dose() {
        // Same reading twice: the integral grows, so the second dose is at least as big
        let mut pid = Pid::new();
        let first = pid.update(5.0, 45.0);
        let second = pid.update(5.0, 45.0);
        assert!(second >= first, "{second} < {first}");
    }

    // ---- flow meter conversion ----
    #[test]
    fn ml_to_pulses_never_zero() {
        assert_eq!(ml_to_pulses(0.0), 1);
        assert_eq!(ml_to_pulses(1000.0), P_FLOW_PULSES_PER_L as u32);
    }

    #[test]
    fn pulses_and_ml_roughly_round_trip() {
        for ml in [50u32, 100, 250, 300] {
            let back = pulses_to_ml(ml_to_pulses(ml as f32));
            // one pulse is 1000 / P_FLOW_PULSES_PER_L mL, so allow that much rounding
            let pulse_ml = (1000.0 / P_FLOW_PULSES_PER_L) as u32 + 1;
            assert!(ml.abs_diff(back) <= pulse_ml, "{ml} mL -> {back} mL");
        }
    }

    // ---- trace codes ----
    #[test]
    fn water_codes_are_distinct_and_in_order() {
        let states = [
            Water::Standby,
            Water::Evaluate { idx: 1, cycle: 2 },
            Water::Active { idx: 1, cycle: 2, sub: ActiveSub::PreControl, start_n: 0, start_s: 0, start_pulses: 0, goal: 0 },
            Water::Active { idx: 1, cycle: 2, sub: ActiveSub::FlowActive, start_n: 0, start_s: 0, start_pulses: 0, goal: 0 },
            Water::Slowdown { idx: 1, cycle: 2, start_n: 0, end: RunEnd::Done },
            Water::Hold { idx: 1, cycle: 2, start_s: 0, reads: 0, last: None, steady: 0, next_sample_s: 0 },
        ];
        for (i, w) in states.iter().enumerate() {
            assert_eq!(w.code() as usize, i, "{w:?}");
        }
        assert_eq!(Water::Standby.pod(), None);
        assert_eq!(Water::Standby.cycle(), 0);
        for w in &states[1..] {
            assert_eq!((w.pod(), w.cycle()), (Some(1), 2), "{w:?}");
        }
    }

    #[test]
    fn fault_codes() {
        assert_eq!(fault_code(None), 0);
        assert_eq!(fault_code(Some(Fault::NoFlow(4))), 1);
        assert_eq!(fault_code(Some(Fault::NotReached(4))), 2);
        assert_eq!(Fault::NotReached(4).pod(), 4);
    }

    #[test]
    fn pid_integral_starts_at_zero_and_grows() {
        let mut pid = Pid::new();
        assert_eq!(pid.integral(), 0.0);
        pid.update(10.0, 40.0);
        assert!(pid.integral() > 0.0 || P_PID_KI == 0.0);
    }

    // ---- next_pod ----
    #[test]
    fn no_installed_pods_means_standby() {
        assert_eq!(next_pod(0, &[false; SENSORS], &mut Pid::new()), Water::Standby);
    }

    #[test]
    fn next_pod_skips_missing_pods() {
        let mut ok = [false; SENSORS];
        ok[2] = true; // only sensor 2 (R1 P3) plugged in
        assert_eq!(next_pod(0, &ok, &mut Pid::new()), Water::Evaluate { idx: 2, cycle: 0 });
        assert_eq!(next_pod(3, &ok, &mut Pid::new()), Water::Standby);
    }

    #[test]
    fn next_pod_resets_the_pid() {
        let mut pid = Pid::new();
        for _ in 0..10 {
            pid.update(40.0, 10.0);
        }
        next_pod(0, &[true; SENSORS], &mut pid);
        let fresh = Pid::new().update(5.0, 45.0);
        assert_eq!(pid.update(5.0, 45.0), fresh);
    }
}
