//! Pot learning (MENU > RELEARN > pick a plant): how many mL of water raise
//! this pod's soil reading by 1 %. Different pot sizes need very different
//! amounts, and the watering PID scales its doses by this number (control.rs).
//!
//! ```text
//!   START      wait for a fresh moisture reading (= starting moisture)
//!              wetter than P_LEARN_MAX_START_PCT -> fail TOO WET (no room to rise)
//!   PRESSURE   pump on, valve closed for P_PRECONTROL_MS
//!   DOSE       valve open until P_LEARN_DOSE_ML went through the flow meter
//!              (or P_PUMP_MAX_S; no flow after P_FLOW_CHECK_S -> fail NO FLOW)
//!   SLOWDOWN   valve closed, pump off P_SLOWDOWN_MS later
//!   SOAK       read every P_HOLD_SAMPLE_S until steady (same rule as watering HOLD)
//!              risen >= P_LEARN_MIN_RISE_PCT -> done: mL given / % risen
//!              else another dose, up to P_LEARN_MAX_DOSES -> fail NO RISE
//! ```
//! Pod unplugged at any point -> fail POD REMOVED (pump stopped safely first).
//!
//! `step()` is called every 100 ms with the inputs and returns what the pump
//! and valve should do; it never touches hardware itself, so it is tested here
//! against a simulated pot.

use crate::control::{ml_to_pulses, pulses_to_ml};
use crate::params::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LearnError {
    Removed, // pod unplugged
    TooWet,  // soil too wet to see a rise
    NoFlow,  // pump ran, flow meter saw nothing
    NoRise,  // all doses given, reading barely moved
    Aborted, // button held
}

impl LearnError {
    /// LCD text, up to 16 characters
    pub fn text(self) -> &'static str {
        match self {
            LearnError::Removed => "POD REMOVED",
            LearnError::TooWet => "SOIL TOO WET",
            LearnError::NoFlow => "NO FLOW",
            LearnError::NoRise => "NO RISE IN SOIL",
            LearnError::Aborted => "ABORTED",
        }
    }
}

/// What happens after SLOWDOWN
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum After {
    Soak,
    Fail(LearnError),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Start,
    Pressure { since: u32 },
    Dose { since: u32, start_pulses: u32, goal: u32 },
    Slowdown { since: u32, then: After },
    Soak { since: u32, last: Option<u8>, steady: u8, next_read: u32 },
    Done(Result<u8, LearnError>), // Ok = learned mL per %
}

impl Phase {
    /// Code for the trace: 1 start, 2 pressure, 3 dose, 4 slowdown, 5 soak, 6 done (0 = not learning)
    pub fn code(&self) -> u8 {
        match self {
            Phase::Start => 1,
            Phase::Pressure { .. } => 2,
            Phase::Dose { .. } => 3,
            Phase::Slowdown { .. } => 4,
            Phase::Soak { .. } => 5,
            Phase::Done(_) => 6,
        }
    }
}

/// Inputs for one step
#[derive(Clone, Copy, Debug)]
pub struct LearnIn {
    pub now_ms: u32,     // any millisecond counter (wrapping is fine)
    pub pulses: u32,     // flow meter total
    pub reads: u32,      // completed moisture readings (changes when a new one is in)
    pub pct: Option<u8>, // this pod's moisture, None = pod not installed
    pub flowing: bool,   // flow meter sees flow right now
}

/// Outputs of one step
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LearnOut {
    pub pump: bool,
    pub valve: bool,
    pub want_read: bool, // take a moisture reading now
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Learn {
    pub idx: usize, // plant slot
    pub phase: Phase,
    pub start_pct: u8,
    pub now_pct: u8,
    pub doses: u8,
    pub pulses_given: u32, // completed doses (the running dose is added when it ends)
    reads: u32,
}

/// Has `now` reached `at`? (wrap-safe)
fn reached(now: u32, at: u32) -> bool {
    now.wrapping_sub(at) as i32 >= 0
}

impl Learn {
    /// Start learning plant slot `idx`. `reads` = the current reading count,
    /// so the first reading taken after this one is the starting moisture.
    pub fn new(idx: usize, reads: u32) -> Self {
        Learn { idx, phase: Phase::Start, start_pct: 0, now_pct: 0, doses: 0, pulses_given: 0, reads }
    }

    pub fn done(&self) -> bool {
        matches!(self.phase, Phase::Done(_))
    }

    /// Stop now (button held). The caller switches the pump and valve off.
    pub fn abort(&mut self) {
        if !self.done() {
            self.phase = Phase::Done(Err(LearnError::Aborted));
        }
    }

    /// mL given so far, including the dose running now
    pub fn ml_given(&self, pulses_now: u32) -> u32 {
        let running = match self.phase {
            Phase::Dose { start_pulses, .. } => pulses_now.wrapping_sub(start_pulses),
            _ => 0,
        };
        pulses_to_ml(self.pulses_given + running)
    }

    pub fn step(&mut self, inp: LearnIn) -> LearnOut {
        let fresh = inp.reads != self.reads;
        self.reads = inp.reads;
        let now = inp.now_ms;
        let mut want_read = false;

        // Pod unplugged: stop the pump the normal way (valve first), then fail
        if inp.pct.is_none() {
            self.phase = match self.phase {
                Phase::Pressure { .. } | Phase::Dose { .. } => {
                    self.end_dose(inp.pulses);
                    Phase::Slowdown { since: now, then: After::Fail(LearnError::Removed) }
                }
                Phase::Slowdown { since, .. } => Phase::Slowdown { since, then: After::Fail(LearnError::Removed) },
                Phase::Done(r) => Phase::Done(r),
                _ => Phase::Done(Err(LearnError::Removed)),
            };
        }
        if let (true, Some(pct)) = (fresh, inp.pct) {
            self.now_pct = pct;
        }

        self.phase = match self.phase {
            Phase::Start => {
                want_read = true;
                match (fresh, inp.pct) {
                    (true, Some(pct)) if pct > P_LEARN_MAX_START_PCT => Phase::Done(Err(LearnError::TooWet)),
                    (true, Some(pct)) => {
                        self.start_pct = pct;
                        Phase::Pressure { since: now }
                    }
                    _ => Phase::Start,
                }
            }
            Phase::Pressure { since } => {
                if now.wrapping_sub(since) >= P_PRECONTROL_MS {
                    self.doses += 1;
                    Phase::Dose { since: now, start_pulses: inp.pulses, goal: ml_to_pulses(P_LEARN_DOSE_ML) }
                } else {
                    self.phase
                }
            }
            Phase::Dose { since, start_pulses, goal } => {
                let ran = now.wrapping_sub(since);
                let delivered = inp.pulses.wrapping_sub(start_pulses);
                if ran >= P_FLOW_CHECK_S * 1000 && !inp.flowing {
                    self.end_dose(inp.pulses);
                    Phase::Slowdown { since: now, then: After::Fail(LearnError::NoFlow) }
                } else if delivered >= goal || ran >= P_PUMP_MAX_S * 1000 {
                    self.end_dose(inp.pulses);
                    Phase::Slowdown { since: now, then: After::Soak }
                } else {
                    self.phase
                }
            }
            Phase::Slowdown { since, then } => {
                if now.wrapping_sub(since) >= P_SLOWDOWN_MS {
                    match then {
                        After::Soak => Phase::Soak { since: now, last: None, steady: 0, next_read: now },
                        After::Fail(e) => Phase::Done(Err(e)),
                    }
                } else {
                    self.phase
                }
            }
            Phase::Soak { since, mut last, mut steady, mut next_read } => {
                if fresh {
                    steady = match last {
                        Some(prev) if self.now_pct.abs_diff(prev) <= P_HOLD_STEADY_PCT => steady + 1,
                        _ => 0,
                    };
                    last = Some(self.now_pct);
                }
                let settled = steady >= P_HOLD_STEADY_COUNT || now.wrapping_sub(since) >= P_HOLD_MAX_S * 1000;
                if settled {
                    self.after_soak(now)
                } else {
                    if reached(now, next_read) {
                        want_read = true;
                        next_read = now.wrapping_add(P_HOLD_SAMPLE_S * 1000);
                    }
                    Phase::Soak { since, last, steady, next_read }
                }
            }
            Phase::Done(r) => Phase::Done(r),
        };

        LearnOut {
            pump: matches!(self.phase, Phase::Pressure { .. } | Phase::Dose { .. } | Phase::Slowdown { .. }),
            valve: matches!(self.phase, Phase::Dose { .. }),
            want_read,
        }
    }

    /// Add the dose that just ended to the total
    fn end_dose(&mut self, pulses_now: u32) {
        if let Phase::Dose { start_pulses, .. } = self.phase {
            self.pulses_given += pulses_now.wrapping_sub(start_pulses);
        }
    }

    fn after_soak(&mut self, now: u32) -> Phase {
        let rise = self.now_pct.saturating_sub(self.start_pct);
        if rise >= P_LEARN_MIN_RISE_PCT {
            let ml = pulses_to_ml(self.pulses_given);
            let ratio = (ml + rise as u32 / 2) / rise as u32; // rounded
            Phase::Done(Ok(ratio.clamp(1, 254) as u8))
        } else if self.doses < P_LEARN_MAX_DOSES {
            Phase::Pressure { since: now } // not enough rise to measure yet: more water
        } else {
            Phase::Done(Err(LearnError::NoRise))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pot on the wall: soil reading = start + mL given / ml_per_pct
    /// (after the water has soaked in, i.e. once the valve is closed).
    struct Pot {
        ml_per_pct: f32,
        start: u8,
        pumps: bool,      // false = pump runs but nothing flows
        unplug_at: u32,   // step number the pod is pulled out (u32::MAX = never)
    }

    struct Sim {
        now: u32,
        pulses: u32,
        reads: u32,
        pct: u8,
        steps: u32,
    }

    fn run(pot: Pot) -> (Learn, Sim) {
        let mut sim = Sim { now: 5_000, pulses: 0, reads: 7, pct: pot.start, steps: 0 };
        let mut learn = Learn::new(1, sim.reads);
        let mut out = LearnOut { pump: false, valve: false, want_read: false };
        while !learn.done() {
            assert!(sim.steps < 100_000, "learning never finished: {:?}", learn.phase);
            // the world reacts to the last outputs
            let flowing = out.pump && out.valve && pot.pumps;
            if flowing {
                sim.pulses += 1; // 10 pulses/s
            }
            if !out.valve {
                let ml = pulses_to_ml(sim.pulses) as f32;
                sim.pct = (pot.start as f32 + ml / pot.ml_per_pct).min(100.0) as u8;
            }
            if out.want_read {
                sim.reads += 1; // the firmware reads within the same 100 ms
            }
            let installed = sim.steps < pot.unplug_at;
            out = learn.step(LearnIn {
                now_ms: sim.now,
                pulses: sim.pulses,
                reads: sim.reads,
                pct: installed.then_some(sim.pct),
                flowing,
            });
            if learn.done() {
                assert!(!out.pump && !out.valve, "outputs must be off when done");
            }
            sim.now = sim.now.wrapping_add(100);
            sim.steps += 1;
        }
        (learn, sim)
    }

    fn pot(ml_per_pct: f32) -> Pot {
        Pot { ml_per_pct, start: 30, pumps: true, unplug_at: u32::MAX }
    }

    fn learned(l: &Learn) -> u8 {
        match l.phase {
            Phase::Done(Ok(r)) => r,
            other => panic!("expected a result, got {other:?}"),
        }
    }

    #[test]
    fn small_pot_learns_its_size_in_one_dose() {
        let (l, _) = run(pot(5.0));
        assert_eq!(l.doses, 1);
        assert!(learned(&l).abs_diff(5) <= 1, "{}", learned(&l));
    }

    #[test]
    fn large_pot_needs_more_doses() {
        // 100 mL raises a 50 mL/% pot by only 2 %: needs a second dose to see 3 %
        let (l, _) = run(pot(50.0));
        assert!(l.doses >= 2, "doses {}", l.doses);
        let r = learned(&l) as f32;
        assert!((r - 50.0).abs() <= 50.0 * 0.15, "learned {r} for a 50 mL/% pot");
    }

    #[test]
    fn soil_that_never_rises_fails() {
        let (l, sim) = run(pot(10_000.0));
        assert_eq!(l.phase, Phase::Done(Err(LearnError::NoRise)));
        assert_eq!(l.doses, P_LEARN_MAX_DOSES);
        assert!(sim.pulses > 0);
    }

    #[test]
    fn no_flow_fails_and_stops_the_pump() {
        let (l, _) = run(Pot { pumps: false, ..pot(5.0) });
        assert_eq!(l.phase, Phase::Done(Err(LearnError::NoFlow)));
    }

    #[test]
    fn wet_soil_is_refused_without_pumping() {
        let (l, sim) = run(Pot { start: P_LEARN_MAX_START_PCT + 1, ..pot(5.0) });
        assert_eq!(l.phase, Phase::Done(Err(LearnError::TooWet)));
        assert_eq!(sim.pulses, 0);
        assert_eq!(l.doses, 0);
    }

    #[test]
    fn unplugging_mid_dose_stops_safely() {
        // pulled out ~1 s after the start, during the dose
        let (l, _) = run(Pot { unplug_at: 10, ..pot(5.0) });
        assert_eq!(l.phase, Phase::Done(Err(LearnError::Removed)));
    }

    #[test]
    fn abort_finishes_at_once() {
        let mut l = Learn::new(0, 0);
        l.abort();
        assert_eq!(l.phase, Phase::Done(Err(LearnError::Aborted)));
        let out = l.step(LearnIn { now_ms: 0, pulses: 0, reads: 1, pct: Some(30), flowing: false });
        assert!(!out.pump && !out.valve);
    }

    #[test]
    fn phase_codes_are_distinct() {
        let phases = [
            Phase::Start,
            Phase::Pressure { since: 0 },
            Phase::Dose { since: 0, start_pulses: 0, goal: 0 },
            Phase::Slowdown { since: 0, then: After::Soak },
            Phase::Soak { since: 0, last: None, steady: 0, next_read: 0 },
            Phase::Done(Ok(1)),
        ];
        for (i, p) in phases.iter().enumerate() {
            assert_eq!(p.code() as usize, i + 1);
        }
    }

    #[test]
    fn error_texts_fit_the_lcd() {
        for e in [LearnError::Removed, LearnError::TooWet, LearnError::NoFlow, LearnError::NoRise, LearnError::Aborted] {
            assert!(e.text().len() <= 16, "{e:?}");
        }
    }

    #[test]
    fn works_across_the_millisecond_counter_wrapping() {
        let mut sim_pot = pot(5.0);
        sim_pot.start = 20;
        // start just before u32::MAX
        let mut learn = Learn::new(1, 0);
        let mut now = u32::MAX - 3_000;
        let (mut pulses, mut reads) = (0u32, 0u32);
        let mut out = LearnOut { pump: false, valve: false, want_read: false };
        let mut pct = sim_pot.start;
        for _ in 0..100_000 {
            if out.pump && out.valve {
                pulses += 1;
            } else {
                pct = (sim_pot.start as f32 + pulses_to_ml(pulses) as f32 / sim_pot.ml_per_pct) as u8;
            }
            if out.want_read {
                reads += 1;
            }
            out = learn.step(LearnIn { now_ms: now, pulses, reads, pct: Some(pct), flowing: out.pump && out.valve });
            if learn.done() {
                break;
            }
            now = now.wrapping_add(100);
        }
        assert!(learned(&learn).abs_diff(5) <= 1);
    }
}
