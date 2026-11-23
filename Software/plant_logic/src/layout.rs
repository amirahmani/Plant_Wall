//! LAYOUT: edit this file when plants, sensors or valves change.
//! Also: which sensor/valve/light belongs to which plant slot.

// Rows on the wall, and how many plants each row has (1..=MAX_PLANTS).
// Final wall: 2 rows of 3. Pods that aren't plugged in are detected at runtime
// (plant_pod_installed) and shown as "--".
pub const ROWS: usize = 2;
pub const MAX_PLANTS: usize = 3;

pub const PLANTS_PER_ROW: [usize; ROWS] = [3, 3];

// Hardware wired (must match the pins set up in main). Whether each sensor
// is actually plugged in is checked at runtime, see plant_pod_installed().
pub const SENSORS: usize = 6; // ADC channels wired
pub const VALVES: usize = 6; // valve outputs set up in main

// Which hygrometer & valve belongs to each plant: Some(index) or None if not wired.
// [row][plant], so SENSOR_OF[0][2] is R1 P3.
pub const SENSOR_OF: [[Option<usize>; MAX_PLANTS]; ROWS] = [
    [Some(0), Some(1), Some(2)],
    [Some(3), Some(4), Some(5)],
];
pub const VALVE_OF: [[Option<usize>; MAX_PLANTS]; ROWS] = [
    [Some(0), Some(1), Some(2)],
    [Some(3), Some(4), Some(5)],
];

// Grow light outputs set up in main, and which light belongs to each plant
pub const LIGHTS: usize = 6;
pub const LIGHT_OF: [[Option<usize>; MAX_PLANTS]; ROWS] = [
    [Some(0), Some(1), Some(2)],
    [Some(3), Some(4), Some(5)],
];

// Compile-time check of the layout above
pub const fn layout_ok() -> bool {
    let mut r = 0;
    while r < ROWS {
        let n = PLANTS_PER_ROW[r];
        if n == 0 || n > MAX_PLANTS {
            return false;
        }
        let mut p = 0;
        while p < MAX_PLANTS {
            if let Some(s) = SENSOR_OF[r][p] {
                if s >= SENSORS || p >= n {
                    return false; // sensor index out of range, or on a plant that doesn't exist
                }
            }
            if let Some(v) = VALVE_OF[r][p] {
                if v >= VALVES || p >= n {
                    return false;
                }
            }
            if let Some(l) = LIGHT_OF[r][p] {
                if l >= LIGHTS || p >= n {
                    return false;
                }
            }
            p += 1;
        }
        r += 1;
    }
    true
}
const _: () = assert!(layout_ok(), "LAYOUT error: check PLANTS_PER_ROW, SENSOR_OF and VALVE_OF");

// Plants are numbered row by row: index = row * MAX_PLANTS + plant
pub const PLANT_SLOTS: usize = ROWS * MAX_PLANTS;

pub fn row_plant(idx: usize) -> (usize, usize) {
    (idx / MAX_PLANTS, idx % MAX_PLANTS)
}

/// A plant pod is installed when the plant exists in the layout, its sensor
/// is wired, detected and calibrated (hygro_ok), and it has a valve.
/// Only installed pods can be edited and watered; the rest show "--".
pub fn plant_pod_installed(row: usize, plant: usize, hygro_ok: &[bool; SENSORS]) -> bool {
    if row >= ROWS || plant >= PLANTS_PER_ROW[row] {
        return false;
    }
    match (SENSOR_OF[row][plant], VALVE_OF[row][plant]) {
        (Some(s), Some(_)) => hygro_ok[s],
        _ => false,
    }
}

/// Sensor and valve of a plant, only if its pod is installed.
pub fn sensor_valve(idx: usize, hygro_ok: &[bool; SENSORS]) -> Option<(usize, usize)> {
    let (r, p) = row_plant(idx);
    if !plant_pod_installed(r, p, hygro_ok) {
        return None;
    }
    match (SENSOR_OF[r][p], VALVE_OF[r][p]) {
        (Some(s), Some(v)) => Some((s, v)),
        _ => None,
    }
}

/// Valve of a plant regardless of the sensor, so a valve can always be closed
/// even if its hygrometer is unplugged mid-watering.
pub fn valve_of(idx: usize) -> Option<usize> {
    let (r, p) = row_plant(idx);
    if r >= ROWS {
        return None;
    }
    VALVE_OF[r][p]
}

/// Valve of a plant that exists in the layout (used by sys check, which tests
/// the plumbing whether or not a hygrometer is plugged in).
pub fn wired_valve(idx: usize) -> Option<usize> {
    let (r, p) = row_plant(idx);
    if r >= ROWS || p >= PLANTS_PER_ROW[r] {
        return None;
    }
    VALVE_OF[r][p]
}

/// Next plant at or after `from` with a wired valve.
pub fn next_wired_valve(from: usize) -> Option<usize> {
    (from..PLANT_SLOTS).find(|&i| wired_valve(i).is_some())
}

/// i-th line on the CALIBRATE screen: (row, plant, sensor), in wall order.
pub fn cal_entry(i: usize) -> Option<(usize, usize, usize)> {
    (0..PLANT_SLOTS)
        .filter_map(|idx| {
            let (r, p) = row_plant(idx);
            if r < ROWS && p < PLANTS_PER_ROW[r] {
                SENSOR_OF[r][p].map(|s| (r, p, s))
            } else {
                None
            }
        })
        .nth(i)
}

/// Number of lines on the CALIBRATE screen.
pub fn cal_count() -> usize {
    (0..PLANT_SLOTS).take_while(|&i| cal_entry(i).is_some()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_OK: [bool; SENSORS] = [true; SENSORS];
    const NONE_OK: [bool; SENSORS] = [false; SENSORS];

    #[test]
    fn row_plant_numbers_row_by_row() {
        assert_eq!(row_plant(0), (0, 0));
        assert_eq!(row_plant(MAX_PLANTS - 1), (0, MAX_PLANTS - 1));
        assert_eq!(row_plant(MAX_PLANTS), (1, 0));
    }

    #[test]
    fn pod_needs_a_working_sensor() {
        assert!(plant_pod_installed(0, 0, &ALL_OK));
        assert!(!plant_pod_installed(0, 0, &NONE_OK));
    }

    #[test]
    fn pod_outside_the_layout_is_not_installed() {
        assert!(!plant_pod_installed(ROWS, 0, &ALL_OK));
        assert!(!plant_pod_installed(0, PLANTS_PER_ROW[0], &ALL_OK));
    }

    #[test]
    fn unplugged_sensor_still_lets_the_valve_close() {
        // sensor_valve hides the pod, but valve_of must still find the valve
        assert_eq!(sensor_valve(0, &NONE_OK), None);
        assert!(valve_of(0).is_some());
    }

    #[test]
    fn every_plant_appears_once_on_the_calibrate_screen() {
        let wired: usize = (0..ROWS)
            .map(|r| (0..PLANTS_PER_ROW[r]).filter(|&p| SENSOR_OF[r][p].is_some()).count())
            .sum();
        assert_eq!(cal_count(), wired);
        assert_eq!(cal_entry(cal_count()), None);
    }

    #[test]
    fn next_wired_valve_runs_out_at_the_end() {
        assert_eq!(next_wired_valve(0), Some(0));
        assert_eq!(next_wired_valve(PLANT_SLOTS), None);
    }
}
