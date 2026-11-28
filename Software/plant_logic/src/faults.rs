//! Every fault the system can detect, how severe it is, and which are active.
//!
//! Severity (1 = worst):
//!   1 CRITICAL  water where it shouldn't be, or the pump running without water moving
//!   2 HIGH      the system can't do its job (tank empty, valve dead, settings not saved)
//!   3 WARNING   one plant isn't being looked after properly
//!   4 INFO      worth knowing, nothing is at risk
//!
//! | Fault      | Sev | Per pod | Active while / cleared by                              |
//! |------------|-----|---------|--------------------------------------------------------|
//! | LEAK       | 1   | no      | SYS CHECK saw flow with every valve shut / next SYS CHECK passes |
//! | NO FLOW    | 1   | yes     | pump ran, flow meter saw nothing / next good dose to that pod |
//! | LOW WATER  | 2   | no      | reservoir level switch open                            |
//! | VALVE BAD  | 2   | yes     | SYS CHECK saw no flow through that valve / next SYS CHECK passes it |
//! | SAVE FAIL  | 2   | no      | last flash save failed / next save works               |
//! | NOT WET    | 3   | yes     | max doses given, still below target / pod reaches target |
//! | NO SENSOR  | 3   | yes     | a sensor that was plugged in since power-on is gone    |
//! | LRN FAIL   | 3   | yes     | pot learning failed / next learning of that pod works  |
//! | OVERRUN    | 4   | no      | a 10 ms tick ran late in the last minute               |
//! | NO CLOCK   | 4   | no      | time of day not set (after a power loss)               |
//!
//! The case LED: fast flash for severity 1-2, slow blink for 3, off for 4 only.

use crate::layout::PLANT_SLOTS;

pub const SEV_CRITICAL: u8 = 1;
pub const SEV_HIGH: u8 = 2;
pub const SEV_WARNING: u8 = 3;
pub const SEV_INFO: u8 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FaultKind {
    Leak,
    NoFlow,
    LowWater,
    ValveBad,
    SaveFail,
    NotWet,
    NoSensor,
    LearnFail,
    Overrun,
    NoClock,
}

/// All faults, most severe first (the FAULTS screen lists them in this order)
pub const FAULT_KINDS: [FaultKind; 10] = [
    FaultKind::Leak,
    FaultKind::NoFlow,
    FaultKind::LowWater,
    FaultKind::ValveBad,
    FaultKind::SaveFail,
    FaultKind::NotWet,
    FaultKind::NoSensor,
    FaultKind::LearnFail,
    FaultKind::Overrun,
    FaultKind::NoClock,
];

impl FaultKind {
    pub fn severity(self) -> u8 {
        match self {
            FaultKind::Leak | FaultKind::NoFlow => SEV_CRITICAL,
            FaultKind::LowWater | FaultKind::ValveBad | FaultKind::SaveFail => SEV_HIGH,
            FaultKind::NotWet | FaultKind::NoSensor | FaultKind::LearnFail => SEV_WARNING,
            FaultKind::Overrun | FaultKind::NoClock => SEV_INFO,
        }
    }

    /// LCD name, up to 9 characters
    pub fn name(self) -> &'static str {
        match self {
            FaultKind::Leak => "LEAK",
            FaultKind::NoFlow => "NO FLOW",
            FaultKind::LowWater => "LOW WATER",
            FaultKind::ValveBad => "VALVE BAD",
            FaultKind::SaveFail => "SAVE FAIL",
            FaultKind::NotWet => "NOT WET",
            FaultKind::NoSensor => "NO SENSOR",
            FaultKind::LearnFail => "LRN FAIL",
            FaultKind::Overrun => "OVERRUN",
            FaultKind::NoClock => "NO CLOCK",
        }
    }

    /// Faults that belong to one plant (shown with its R#P#)
    pub fn per_pod(self) -> bool {
        matches!(self, FaultKind::NoFlow | FaultKind::ValveBad | FaultKind::NotWet | FaultKind::NoSensor | FaultKind::LearnFail)
    }

    /// Code for the trace: position in FAULT_KINDS + 1 (0 = no fault)
    pub fn code(self) -> u8 {
        self.index() as u8 + 1
    }

    fn index(self) -> usize {
        FAULT_KINDS.iter().position(|&k| k == self).unwrap_or(0)
    }
}

/// One line of the FAULTS screen
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FaultLine {
    ActiveHeader(usize),                 // "ACTIVE: n"
    Active(FaultKind, Option<usize>),    // with its plant slot for per-pod faults
    InactiveHeader(usize),               // "INACTIVE: n"
    Inactive(FaultKind),
}

/// Which faults are active. Per fault: one bit per plant slot (per-pod faults)
/// or bit 0 (system faults).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Faults {
    bits: [u8; FAULT_KINDS.len()],
}

const _: () = assert!(PLANT_SLOTS <= 8, "fault bits hold up to 8 plant slots");

impl Faults {
    pub const fn new() -> Self {
        Faults { bits: [0; FAULT_KINDS.len()] }
    }

    fn mask(kind: FaultKind, pod: Option<usize>) -> u8 {
        match (kind.per_pod(), pod) {
            (true, Some(p)) if p < PLANT_SLOTS => 1 << p,
            (true, _) => 0, // per-pod fault without a valid pod: ignored
            (false, _) => 1,
        }
    }

    /// Switch a fault on or off. `pod` is the plant slot for per-pod faults.
    pub fn set(&mut self, kind: FaultKind, pod: Option<usize>, on: bool) {
        let m = Self::mask(kind, pod);
        let b = &mut self.bits[kind.index()];
        if on {
            *b |= m;
        } else {
            *b &= !m;
        }
    }

    pub fn is_active(&self, kind: FaultKind, pod: Option<usize>) -> bool {
        let m = Self::mask(kind, pod);
        m != 0 && self.bits[kind.index()] & m != 0
    }

    /// Does this plant slot have any per-pod fault? (FL on the main screen)
    pub fn pod_has_fault(&self, pod: usize) -> bool {
        FAULT_KINDS.iter().any(|&k| k.per_pod() && self.is_active(k, Some(pod)))
    }

    /// i-th active fault, most severe first
    pub fn active(&self, i: usize) -> Option<(FaultKind, Option<usize>)> {
        self.active_iter().nth(i)
    }

    pub fn active_count(&self) -> usize {
        self.active_iter().count()
    }

    /// Most severe active fault
    pub fn top(&self) -> Option<(FaultKind, Option<usize>)> {
        self.active(0)
    }

    fn active_iter(&self) -> impl Iterator<Item = (FaultKind, Option<usize>)> + '_ {
        FAULT_KINDS.iter().flat_map(move |&k| {
            let pods = if k.per_pod() { PLANT_SLOTS } else { 1 };
            (0..pods).filter_map(move |p| {
                let pod = if k.per_pod() { Some(p) } else { None };
                self.is_active(k, pod).then_some((k, pod))
            })
        })
    }

    /// Faults that aren't active anywhere
    fn inactive_iter(&self) -> impl Iterator<Item = FaultKind> + '_ {
        FAULT_KINDS.iter().copied().filter(move |&k| self.bits[k.index()] == 0)
    }

    /// i-th line of the FAULTS screen: "ACTIVE: n", the active faults,
    /// "INACTIVE: n", then every fault that isn't active.
    pub fn line(&self, i: usize) -> Option<FaultLine> {
        let active = self.active_count();
        let inactive = self.inactive_iter().count();
        if i == 0 {
            Some(FaultLine::ActiveHeader(active))
        } else if i <= active {
            self.active(i - 1).map(|(k, p)| FaultLine::Active(k, p))
        } else if i == active + 1 {
            Some(FaultLine::InactiveHeader(inactive))
        } else {
            self.inactive_iter().nth(i - active - 2).map(FaultLine::Inactive)
        }
    }

    pub fn line_count(&self) -> usize {
        2 + self.active_count() + self.inactive_iter().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_listed_most_severe_first() {
        for w in FAULT_KINDS.windows(2) {
            assert!(w[0].severity() <= w[1].severity(), "{:?} before {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn names_fit_and_codes_are_unique() {
        for (i, k) in FAULT_KINDS.iter().enumerate() {
            assert!(!k.name().is_empty() && k.name().len() <= 9, "{k:?}");
            assert_eq!(k.code() as usize, i + 1);
            assert!((SEV_CRITICAL..=SEV_INFO).contains(&k.severity()));
        }
    }

    #[test]
    fn nothing_active_at_start() {
        let f = Faults::new();
        assert_eq!(f.active_count(), 0);
        assert_eq!(f.top(), None);
        assert_eq!(f.line(0), Some(FaultLine::ActiveHeader(0)));
        assert_eq!(f.line(1), Some(FaultLine::InactiveHeader(FAULT_KINDS.len())));
        assert_eq!(f.line_count(), 2 + FAULT_KINDS.len());
    }

    #[test]
    fn per_pod_faults_are_separate_per_pod() {
        let mut f = Faults::new();
        f.set(FaultKind::NoFlow, Some(2), true);
        assert!(f.is_active(FaultKind::NoFlow, Some(2)));
        assert!(!f.is_active(FaultKind::NoFlow, Some(1)));
        assert!(f.pod_has_fault(2));
        assert!(!f.pod_has_fault(1));
        f.set(FaultKind::NoFlow, Some(2), false);
        assert_eq!(f.active_count(), 0);
    }

    #[test]
    fn system_faults_ignore_the_pod() {
        let mut f = Faults::new();
        f.set(FaultKind::LowWater, Some(3), true);
        assert!(f.is_active(FaultKind::LowWater, None));
        assert!(!f.pod_has_fault(3), "a system fault isn't a pod fault");
        assert_eq!(f.top(), Some((FaultKind::LowWater, None)));
    }

    #[test]
    fn active_list_is_in_severity_order() {
        let mut f = Faults::new();
        f.set(FaultKind::NoClock, None, true); // 4
        f.set(FaultKind::NotWet, Some(4), true); // 3
        f.set(FaultKind::NotWet, Some(0), true); // 3
        f.set(FaultKind::NoFlow, Some(5), true); // 1
        f.set(FaultKind::LowWater, None, true); // 2
        let got: [_; 5] = core::array::from_fn(|i| f.active(i).unwrap());
        assert_eq!(
            got,
            [
                (FaultKind::NoFlow, Some(5)),
                (FaultKind::LowWater, None),
                (FaultKind::NotWet, Some(0)),
                (FaultKind::NotWet, Some(4)),
                (FaultKind::NoClock, None),
            ]
        );
        assert_eq!(f.active(5), None);
    }

    #[test]
    fn screen_lists_active_then_inactive() {
        let mut f = Faults::new();
        f.set(FaultKind::NotWet, Some(1), true);
        f.set(FaultKind::Leak, None, true);
        assert_eq!(f.line(0), Some(FaultLine::ActiveHeader(2)));
        assert_eq!(f.line(1), Some(FaultLine::Active(FaultKind::Leak, None)));
        assert_eq!(f.line(2), Some(FaultLine::Active(FaultKind::NotWet, Some(1))));
        assert_eq!(f.line(3), Some(FaultLine::InactiveHeader(FAULT_KINDS.len() - 2)));
        assert_eq!(f.line(4), Some(FaultLine::Inactive(FaultKind::NoFlow)));
        // every fault appears exactly once, active or inactive
        let n = f.line_count();
        assert_eq!(f.line(n - 1), Some(FaultLine::Inactive(FaultKind::NoClock)));
        assert_eq!(f.line(n), None);
    }

    #[test]
    fn per_pod_fault_without_a_pod_is_ignored() {
        let mut f = Faults::new();
        f.set(FaultKind::NoFlow, None, true);
        f.set(FaultKind::NoFlow, Some(PLANT_SLOTS), true);
        assert_eq!(f.active_count(), 0);
    }
}
