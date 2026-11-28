//! MANUAL control (MENU > MANUAL): every input and output, one per line.
//! Inputs show their raw value live; outputs can be switched ON / OFF by hand.
//! While this screen is open, automatic watering, lights and the fault LED
//! leave the outputs alone. Leaving it switches every output off again.

use crate::layout::{row_plant, LIGHTS, LIGHT_OF, MAX_PLANTS, PLANTS_PER_ROW, ROWS, SENSORS, SENSOR_OF, VALVES, VALVE_OF};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ManualItem {
    // inputs
    JoyX,
    JoyY,
    Button,
    LevelSwitch,
    FlowPulses,
    Hygro(usize), // sensor number
    // outputs
    Pump,
    Valve(usize), // valve number
    Light(usize), // light number
    Led,
}

const FIXED_INPUTS: usize = 5;
pub const MANUAL_COUNT: usize = FIXED_INPUTS + SENSORS + 1 + VALVES + LIGHTS + 1;

/// i-th line of the MANUAL screen: inputs first, then outputs
pub fn manual_item(i: usize) -> Option<ManualItem> {
    let fixed = [ManualItem::JoyX, ManualItem::JoyY, ManualItem::Button, ManualItem::LevelSwitch, ManualItem::FlowPulses];
    let mut i = i;
    if i < FIXED_INPUTS {
        return Some(fixed[i]);
    }
    i -= FIXED_INPUTS;
    if i < SENSORS {
        return Some(ManualItem::Hygro(i));
    }
    i -= SENSORS;
    if i == 0 {
        return Some(ManualItem::Pump);
    }
    i -= 1;
    if i < VALVES {
        return Some(ManualItem::Valve(i));
    }
    i -= VALVES;
    if i < LIGHTS {
        return Some(ManualItem::Light(i));
    }
    i -= LIGHTS;
    if i == 0 {
        return Some(ManualItem::Led);
    }
    None
}

impl ManualItem {
    pub fn is_output(self) -> bool {
        matches!(self, ManualItem::Pump | ManualItem::Valve(_) | ManualItem::Light(_) | ManualItem::Led)
    }
}

/// Which plant (row, plant) a sensor / valve / light number belongs to
fn owner(table: &[[Option<usize>; MAX_PLANTS]; ROWS], n: usize) -> Option<(usize, usize)> {
    (0..ROWS * MAX_PLANTS).map(row_plant).find(|&(r, p)| p < PLANTS_PER_ROW[r] && table[r][p] == Some(n))
}

/// Short name, up to 8 characters, e.g. "VLV R1P2" (or "VLV 3" if not on a plant)
pub fn manual_label(item: ManualItem, out: &mut impl core::fmt::Write) {
    let (prefix, plant, n) = match item {
        ManualItem::JoyX => ("JOY X", None, None),
        ManualItem::JoyY => ("JOY Y", None, None),
        ManualItem::Button => ("BUTTON", None, None),
        ManualItem::LevelSwitch => ("LEVEL SW", None, None),
        ManualItem::FlowPulses => ("FLOW", None, None),
        ManualItem::Pump => ("PUMP", None, None),
        ManualItem::Led => ("LED", None, None),
        ManualItem::Hygro(s) => ("HYG", owner(&SENSOR_OF, s), Some(s)),
        ManualItem::Valve(v) => ("VLV", owner(&VALVE_OF, v), Some(v)),
        ManualItem::Light(l) => ("LGT", owner(&LIGHT_OF, l), Some(l)),
    };
    let _ = match (plant, n) {
        (Some((r, p)), _) => write!(out, "{} R{}P{}", prefix, r + 1, p + 1),
        (None, Some(n)) => write!(out, "{} {}", prefix, n + 1),
        (None, None) => out.write_str(prefix),
    };
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;

    fn label(item: ManualItem) -> String {
        let mut s = String::new();
        manual_label(item, &mut s);
        s
    }

    #[test]
    fn list_has_every_input_and_output_once() {
        let items: std::vec::Vec<_> = (0..MANUAL_COUNT).map(|i| manual_item(i).unwrap()).collect();
        assert_eq!(manual_item(MANUAL_COUNT), None);
        for s in 0..SENSORS {
            assert_eq!(items.iter().filter(|&&i| i == ManualItem::Hygro(s)).count(), 1);
        }
        for v in 0..VALVES {
            assert_eq!(items.iter().filter(|&&i| i == ManualItem::Valve(v)).count(), 1);
        }
        for l in 0..LIGHTS {
            assert_eq!(items.iter().filter(|&&i| i == ManualItem::Light(l)).count(), 1);
        }
        assert_eq!(items.iter().filter(|i| i.is_output()).count(), 1 + VALVES + LIGHTS + 1);
    }

    #[test]
    fn inputs_come_before_outputs() {
        let first_output = (0..MANUAL_COUNT).position(|i| manual_item(i).unwrap().is_output()).unwrap();
        assert!((first_output..MANUAL_COUNT).all(|i| manual_item(i).unwrap().is_output()));
    }

    #[test]
    fn labels_name_the_plant_and_fit() {
        assert_eq!(label(ManualItem::Hygro(0)), "HYG R1P1");
        assert_eq!(label(ManualItem::Valve(4)), "VLV R2P2");
        assert_eq!(label(ManualItem::Light(5)), "LGT R2P3");
        for i in 0..MANUAL_COUNT {
            let l = label(manual_item(i).unwrap());
            assert!(!l.is_empty() && l.len() <= 8, "{l:?}");
        }
    }
}
