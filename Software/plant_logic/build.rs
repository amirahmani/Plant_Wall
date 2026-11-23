//! Build script: turns params.dcm into Rust constants at compile time.
//!
//! Supports:
//!   FESTWERT name            -> pub const NAME: T = value;
//!   FESTWERTEBLOCK name N    -> pub const NAME: [T; N] = [...];
//! Other lines (LANGNAME, EINHEIT_W, comments starting with '*', etc.) are ignored.
//!
//! Generated file: $OUT_DIR/params.rs, pulled into main.rs with include!().
//! Cargo re-runs this whenever params.dcm changes.

use std::{env, fs, path::Path};

const PARAM_FILE: &str = "params.dcm";

// Rust type for each parameter. DCM files carry no type info, so anything not
// listed here defaults to i32 for whole numbers and f32 if any value has a decimal.
const TYPES: &[(&str, &str)] = &[
    ("P_Joystick_LOW", "u16"),
    ("P_Joystick_HIGH", "u16"),
    ("P_BUTTON_COOLDOWN", "u8"),
    ("P_MOISTURE_DRY", "u16"),
    ("P_MOISTURE_WET", "u16"),
    ("P_MOISTURE_SAMPLES", "u32"),
    ("P_SENSOR_WARMUP_MS", "u32"),
    ("P_MOISTURE_PERIOD_S", "u32"),
    ("P_FLOW_PULSES_PER_L", "f32"),
    ("P_WATER_INTERVAL_S", "u32"),
    ("P_PUMP_CYCLE_S", "u32"),
    ("P_SOAK_S", "u32"),
    ("P_MAX_WATER_CYCLES", "u8"),
    ("P_FLOW_CHECK_S", "u32"),
    ("P_TARGET_MOISTURE", "u8"),
    ("P_WATER_INTERVAL_S", "u32"),
    ("P_TARGET_MOISTURE", "u8"),
    ("P_WATER_STAGES", "u8"),
    ("P_ML_PER_PCT", "f32"),
    ("P_PUMP_MAX_S", "u32"),
    ("P_SOAK_S", "u32"),
    ("P_FLOW_CHECK_S", "u32"),
    ("P_HYGRO_MIN_RAW", "u16"),
    ("P_HYGRO_MAX_RAW", "u16"),
    ("P_LONG_PRESS_MS", "u32"),
    ("P_SYSCHK_LED_MS", "u32"),
    ("P_SYSCHK_STEP_MS", "u32"),
    ("P_SYSCHK_MIN_LPM", "f32"),
    ("P_SYSCHK_MAX_LPM", "f32"),
    ("P_LIGHT_HOURS", "u8"), 
    ("P_LIGHT_START_H", "u8"),
    ("P_JOY_MAX_OFFSET", "u16"),
    ("P_FS_JOY_RESET", "u8"),
    ("P_PID_KP", "f32"), 
    ("P_PID_KI", "f32"), 
    ("P_PID_KD", "f32"), 
    ("P_PID_I_MAX", "f32"),
    ("P_DOSE_MIN_ML", "f32"), 
    ("P_DOSE_MAX_ML", "f32"),
    ("P_DEADBAND_PCT", "u8"), 
    ("P_MAX_CYCLES", "u8"),
    ("P_PRECONTROL_MS", "u32"),
    ("P_SLOWDOWN_MS", "u32"),
    ("P_HOLD_SAMPLE_S", "u32"),
    ("P_HOLD_STEADY_PCT", "u8"),
    ("P_HOLD_STEADY_COUNT", "u8"), 
    ("P_HOLD_MAX_S", "u32"),
    ("P_LEVEL_DEBOUNCE_MS", "u32")
];

struct Entry {
    name: String,
    block_len: Option<usize>, // Some(N) for FESTWERTEBLOCK, None for FESTWERT
    values: Vec<String>,
}

fn main() {
    println!("cargo:rerun-if-changed={PARAM_FILE}");

    let text = fs::read_to_string(PARAM_FILE)
        .unwrap_or_else(|e| panic!("could not read {PARAM_FILE}: {e}"));

    let mut out = String::from("// Generated from params.dcm by build.rs. Do not edit.\n\n");
    let mut current: Option<Entry> = None;

    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('*') {
            continue;
        }
        let mut words = line.split_whitespace();
        match words.next() {
            Some("FESTWERT") => {
                current = Some(Entry {
                    name: words.next().expect("FESTWERT without a name").to_string(),
                    block_len: None,
                    values: Vec::new(),
                });
            }
            Some("FESTWERTEBLOCK") => {
                let name = words.next().expect("FESTWERTEBLOCK without a name").to_string();
                let len = words
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| panic!("{PARAM_FILE}:{}: block {name} needs a size", line_no + 1));
                current = Some(Entry { name, block_len: Some(len), values: Vec::new() });
            }
            Some("WERT") => {
                if let Some(entry) = current.as_mut() {
                    entry.values.extend(words.map(String::from));
                }
            }
            Some("END") => {
                if let Some(entry) = current.take() {
                    out.push_str(&emit(&entry));
                }
            }
            _ => {} // LANGNAME, EINHEIT_W, FUNKTION, KONSERVIERUNG_FORMAT, ...
        }
    }

    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("params.rs");
    fs::write(dest, out).unwrap();
}

fn emit(entry: &Entry) -> String {
    let name = &entry.name;

    if entry.values.is_empty() {
        // e.g. a FESTWERT holding TEXT instead of WERT
        println!("cargo:warning={name} in {PARAM_FILE} has no numeric WERT values, skipped");
        return String::new();
    }

    let is_float_value = |v: &str| v.contains(['.', 'e', 'E']);

    let ty = TYPES
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, t)| *t)
        .unwrap_or(if entry.values.iter().any(|v| is_float_value(v)) { "f32" } else { "i32" });
    let float_ty = ty.starts_with('f');

    let lit = |v: &String| -> String {
        if !float_ty && is_float_value(v) {
            panic!("{name} in {PARAM_FILE} is {ty} but has non-integer value {v}");
        }
        if float_ty && !is_float_value(v) {
            format!("{v}.0") // Rust needs 3.0, not 3, for floats
        } else {
            v.clone()
        }
    };

    match entry.block_len {
        None => format!("pub const {name}: {ty} = {};\n", lit(&entry.values[0])),
        Some(n) => {
            if entry.values.len() != n {
                panic!(
                    "{name} in {PARAM_FILE} declares {n} values but has {}",
                    entry.values.len()
                );
            }
            let items: Vec<String> = entry.values.iter().map(lit).collect();
            format!("pub const {name}: [{ty}; {n}] = [{}];\n", items.join(", "))
        }
    }
}
