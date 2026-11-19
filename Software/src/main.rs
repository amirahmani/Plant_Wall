

/*
Hardware setup:
  STM32F446RE Nucleo board
    VCC 3.3V
----------------------------------------------------------------------

HMI:

Joystick
  - Joystick X connected to PA0 / Arduino A0
  - Joystick Y connected to PA1 / Arduino A1
  - Button connected to PA10 (internal pull-up) / Arduino D2
  - 5V input
LED's
  - LED HMI (fault blink, sys check) connected to PB5 / Arduino D4
LCD - I2C 16x2
  - SCL connected to PB8 / Arduino D15
  - SDA connected to PB9 / Arduino D14
  - 3.3V input
----------------------------------------------------------------------

Control Loop:


Wall layout: 2 rows, 3 plants per row (6 plant pods)

Moisture sensors (capacitive v1.2/v2.0 with TLC555, AOUT pin, powered from 3.3V through the switch below)
  Each AOUT input needs a 100k pull-down resistor to GND, so an unplugged
  sensor reads ~0 instead of floating (used by plant_pod_installed).
  - Sensor 0 (Row 1 Plant 1) connected to PA4 / Arduino A2
  - Sensor 1 (Row 1 Plant 2) connected to PB0 / Arduino A3
  - Sensor 2 (Row 1 Plant 3) connected to PC1 / Arduino A4
  - Sensor 3 (Row 2 Plant 1) connected to PC0 / Arduino A5
  - Sensor 4 (Row 2 Plant 2) connected to PB1 (Morpho CN10)
  - Sensor 5 (Row 2 Plant 3) connected to PC2 (Morpho CN7)
  - Sensor power: PB3 / Arduino D3 -> P-MOSFET gate (LOW = sensors on), shared by all
  - Spare analog pins: PC3, PC4, PC5, PA5, PA6, PA7.
    PA2/PA3 are the ST-LINK USB serial, keep them free.
Flow meter - hall effect, 5V powered
  - Signal connected to PC8 (through 10k/20k divider)
  - 5V input
Valves (GPIO out, HIGH = open)
  - Valve Row 1 Plant 1 connected to PB4 / Arduino D5 w/ MOSFET
  - Valve Row 1 Plant 2 connected to PB10 / Arduino D6 w/ MOSFET
  - Valve Row 1 Plant 3 connected to PA8 / Arduino D7 w/ MOSFET
  - Valve Row 2 Plant 1 connected to PA9 / Arduino D8 w/ MOSFET
  - Valve Row 2 Plant 2 connected to PC7 / Arduino D9 w/ MOSFET
  - Valve Row 2 Plant 3 connected to PB6 / Arduino D10 w/ MOSFET
  - 12V input, flyback diode across each valve coil
Motor / pump (GPIO out, HIGH = on)
  - Motor connected to PC6 (Morpho CN10) w/ MOSFET
  - 12V input
----------------------------------------------------------------------

Controls:
  Click (short press)  Browse: edit plant (installed pods only) / Edit: done / Menus: select
  Hold 1 s             Browse/Edit: open MENU / any menu screen: back to browse
                       during SYS CHECK: abort (pump off, all valves closed)
  MENU (up/down to pick, click to select, left to go back):
    RELEARN     re-read all hygrometers now and show which pods are found
    SYS CHECK   LED 1 s, pump alone 0.5 s (leak check), then each valve 0.5 s with flow check
    CLOCK       set the time of day (lost on power loss; until set, watering
                falls back to every P_WATER_INTERVAL_S from power-up)
    WATER TIME  daily watering time (saved to flash)

Which plant each sensor and valve belongs to is set in the LAYOUT section below.
Tuning values (thresholds, calibration, timing) come from params.dcm.

Plant Wtr/Lt settings and the water time are saved to flash on every change (see settings.rs);
memory.x must have FLASH LENGTH = 384K so code stays out of the settings sector.
*/

#![deny(unsafe_code)]
#![allow(clippy::empty_loop)]
#![no_main]
#![no_std]

mod lcd;
//Allow plant settings to be saved
mod settings;

// Adding the parameter file to tune from
#[allow(dead_code, non_upper_case_globals)]
mod params {
    include!(concat!(env!("OUT_DIR"), "/params.rs"));
}

// Halt on panic
use panic_halt as _; // panic handler

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m::interrupt::Mutex;
use cortex_m::peripheral::DWT;
use cortex_m_rt::entry;
// Defining the pins for the STM32
use stm32f4xx_hal::{
    self as hal,
    adc::{config::{AdcConfig, SampleTime}, Adc},
    gpio::{Edge, ExtiPin, Pin, PinState},
    rcc::Config,
};
use crate::hal::{pac, pac::interrupt, prelude::*};
use crate::lcd::Lcd;
use crate::params::*;
use crate::settings::{Store, CELLS, EXTRA};

// LCD constants
const LCD_ADDR: u8 = 0x27;

// ---------------------------------------------------------------
// LAYOUT: edit this section when plants, sensors or valves change
// ---------------------------------------------------------------

// Rows on the wall, and how many plants each row has (1..=MAX_PLANTS).
// Final wall: 2 rows of 3. Pods that aren't plugged in are detected at runtime
// (plant_pod_installed) and shown as "--".
const ROWS: usize = 2;
const MAX_PLANTS: usize = 3;

const PLANTS_PER_ROW: [usize; ROWS] = [3, 3];

// Hardware wired (must match the pins set up in main). Whether each sensor
// is actually plugged in is checked at runtime, see plant_pod_installed().
const SENSORS: usize = 6; // ADC channels wired
const VALVES: usize = 6;  // valve outputs set up in main

// Sensors that have calibration in params.dcm. Sensors past this count are
// treated as not installed until their dry/wet values are added.
const CALIBRATED: usize = P_MOISTURE_DRY.len();

// Which hygrometer & valve belongs to each plant: Some(index) or None if not wired.
// [row][plant], so SENSOR_OF[0][2] is R1 P3.
const SENSOR_OF: [[Option<usize>; MAX_PLANTS]; ROWS] = [
    [Some(0), Some(1), Some(2)],
    [Some(3), Some(4), Some(5)],
];
const VALVE_OF: [[Option<usize>; MAX_PLANTS]; ROWS] = [
    [Some(0), Some(1), Some(2)],
    [Some(3), Some(4), Some(5)],
];

// Compile-time check of the layout above
const fn layout_ok() -> bool {
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
            p += 1;
        }
        r += 1;
    }
    true
}
const _: () = assert!(layout_ok(), "LAYOUT error: check PLANTS_PER_ROW, SENSOR_OF and VALVE_OF");

// LCD columns where the Wtr and Lt values start on line 2 ("R1 P1 LOW LOW")
// PARAMETER!! (move when .dcm integrated)
const WATER_COL: u8 = 6;
const LIGHT_COL: u8 = 10;

// LCD columns of the hour and minute in the time editor ("     07:30")
const HOUR_COL: u8 = 5;
const MIN_COL: u8 = 8;

// Flow below this (L/min) counts as "no flow" (display and pump fault check)
// PARAMETER!! (move when .dcm integrated)
const FLOW_SHOW_MIN_LPM: f32 = 0.05;

// Daily water time used until one is saved from the WATER TIME menu
const DEFAULT_WATER_TIME: (u8, u8) = (7, 0);

const SECONDS_PER_DAY: u32 = 86_400;

// Catch params.dcm mistakes at compile time. The array length can be smaller than
// the physical sensor count when some probes are not fitted yet; we still keep
// the ADC channels wired in place and only use the valid calibration range.
const _: () = assert!(P_MOISTURE_DRY.len() == P_MOISTURE_WET.len(), "MOISTURE_DRY and MOISTURE_WET must have the same length");
const _: () = assert!(P_MOISTURE_DRY.len() <= SENSORS, "MOISTURE calibration arrays cannot exceed the sensor count");
const _: () = assert!(P_MOISTURE_DRY.len() > 0, "MOISTURE calibration arrays must not be empty");
const _: () = assert!(P_MOISTURE_SAMPLES > 0, "MOISTURE_SAMPLES must be at least 1");
const _: () = assert!(P_TARGET_MOISTURE.len() == 3, "TARGET_MOISTURE needs LOW, MED, HI");
const _: () = assert!(P_WATER_STAGES.len() > 0, "WATER_STAGES needs at least one stage");
const _: () = assert!(P_SYSCHK_STEP_MS > 0, "SYSCHK_STEP_MS must be above 0");

// ---------------------------------------------------------------
// Flow meter: counted in the EXTI interrupt
// ---------------------------------------------------------------

// Total pulses since power-on. Atomic, so the interrupt and main loop can share it safely.
static FLOW_PULSES: AtomicU32 = AtomicU32::new(0);

// The pin is kept here so the interrupt can clear its pending flag.
static FLOW_PIN: Mutex<RefCell<Option<Pin<'C', 8>>>> = Mutex::new(RefCell::new(None));

// PC8 is on EXTI line 8, which shares the EXTI9_5 interrupt (lines 5..9)
#[interrupt]
fn EXTI9_5() {
    FLOW_PULSES.fetch_add(1, Ordering::Relaxed);
    cortex_m::interrupt::free(|cs| {
        if let Some(pin) = FLOW_PIN.borrow(cs).borrow_mut().as_mut() {
            pin.clear_interrupt_pending_bit();
        }
    });
}

// Unmasking an interrupt in the NVIC is marked unsafe because it can break
// code that assumes it won't be interrupted. The handler above only touches an
// atomic and a Mutex, so it's safe here; this is the one place unsafe is allowed.
#[allow(unsafe_code)]
fn enable_flow_interrupt() {
    unsafe { cortex_m::peripheral::NVIC::unmask(pac::Interrupt::EXTI9_5) };
}

// ---------------------------------------------------------------
// Hygrometers
// ---------------------------------------------------------------

// Average several ADC readings of one pin. A macro because each pin is a different type.
macro_rules! read_avg {
    ($adc:expr, $pin:expr) => {{
        // First conversion after switching channels can carry charge from the
        // previous channel: discard it
        let _: u16 = $adc.convert(&$pin, SampleTime::Cycles_480);
        let mut sum: u32 = 0;
        for _ in 0..P_MOISTURE_SAMPLES {
            let sample: u16 = $adc.convert(&$pin, SampleTime::Cycles_480);
            sum += sample as u32;
        }
        (sum / P_MOISTURE_SAMPLES) as u16
    }};
}

/// Is a hygrometer actually plugged in on this channel?
///   Unplugged sensor: the 100k pull-down holds the pin near 0    -> below P_HYGRO_MIN_RAW
///   Faulty / out-of-range output (above the reading in air)       -> above P_HYGRO_MAX_RAW
/// Anything in between is a real soil reading.
/// (A capacitive sensor pulled out of the soil reads as very dry, not as unplugged.)
fn hygro_detected(raw: u16) -> bool {
    raw >= P_HYGRO_MIN_RAW && raw <= P_HYGRO_MAX_RAW
}

/// A plant pod is installed when the plant exists in the layout, its hygrometer
/// is wired, calibrated and currently detected, and it has a valve.
/// Only installed pods can be edited and watered; the rest show "--".
fn plant_pod_installed(row: usize, plant: usize, hygro_ok: &[bool; SENSORS]) -> bool {
    if row >= ROWS || plant >= PLANTS_PER_ROW[row] {
        return false;
    }
    match (SENSOR_OF[row][plant], VALVE_OF[row][plant]) {
        (Some(s), Some(_)) => s < CALIBRATED && hygro_ok[s],
        _ => false,
    }
}

/// Raw ADC reading -> 0..100 % moisture, using that sensor's dry/wet calibration.
fn moisture_pct(raw: u16, dry: u16, wet: u16) -> u8 {
    let (raw, dry, wet) = (raw as i32, dry as i32, wet as i32);
    if dry == wet {
        return 0; // uncalibrated, avoid dividing by zero
    }
    let pct = (dry - raw) * 100 / (dry - wet);
    pct.clamp(0, 100) as u8
}

// ---------------------------------------------------------------
// Plant adjustable settings
// ---------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum WaterLevel {
    Low,
    Med,
    High,
}

// Thinking of changing to an actual value instaed of low/med/high, need more info from the plants themselves
impl WaterLevel {
    fn as_str(self) -> &'static str {
        match self {
            WaterLevel::Low => "LOW",
            WaterLevel::Med => "MED",
            WaterLevel::High => "HI",
        }
    }

    // One step up, stops at High
    fn up(self) -> Self {
        match self {
            WaterLevel::Low => WaterLevel::Med,
            WaterLevel::Med | WaterLevel::High => WaterLevel::High,
        }
    }

    // One step down, stops at Low
    fn down(self) -> Self {
        match self {
            WaterLevel::High => WaterLevel::Med,
            WaterLevel::Med | WaterLevel::Low => WaterLevel::Low,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum LightLevel {
    Low,
    Med,
    High,
}

impl LightLevel {
    fn as_str(self) -> &'static str {
        match self {
            LightLevel::Low => "LOW",
            LightLevel::Med => "MED",
            LightLevel::High => "HI",
        }
    }

    fn up(self) -> Self {
        match self {
            LightLevel::Low => LightLevel::Med,
            LightLevel::Med | LightLevel::High => LightLevel::High,
        }
    }

    fn down(self) -> Self {
        match self {
            LightLevel::High => LightLevel::Med,
            LightLevel::Med | LightLevel::Low => LightLevel::Low,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    Center,
    Up,
    Down,
    Left,
    Right,
}

fn read_dir(x: u16, y: u16) -> Dir {
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

// Which value is being edited
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Water,
    Light,
}

impl Field {
    fn other(self) -> Self {
        match self {
            Field::Water => Field::Light,
            Field::Light => Field::Water,
        }
    }
}

// ---------------------------------------------------------------
// Menu / screens
// ---------------------------------------------------------------

const MENU: [&str; 5] = ["RELEARN", "SYS CHECK", "CLOCK", "WATER TIME", "EXIT"];
const MENU_RELEARN: usize = 0;
const MENU_SYSCHECK: usize = 1;
const MENU_CLOCK: usize = 2;
const MENU_WATER_TIME: usize = 3;
// MENU[4] = EXIT: any other item goes back to browsing

/// Which time the time editor is setting
#[derive(Clone, Copy, PartialEq)]
enum TimeSet {
    Clock,
    Water,
}

/// Hours or minutes selected in the time editor
#[derive(Clone, Copy, PartialEq)]
enum TimeField {
    Hour,
    Minute,
}

/// Sys check sequence steps
#[derive(Clone, Copy, PartialEq)]
enum Sys {
    Led,          // HMI LED on
    PumpOnly,     // pump on, all valves closed: flow here means a leak
    Valve(usize), // pump on, this plant's valve open: flow must look real
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Browse,
    Edit(Field),
    Menu(usize),
    Relearn { reads: u32 }, // waiting for a fresh hygrometer reading
    RelearnResult,
    SysCheck { step: Sys, start_n: u32, start_pulses: u32 },
    SysResult,
    SetTime { what: TimeSet, hh: u8, mm: u8, field: TimeField },
}

// ---------------------------------------------------------------
// Saving plant settings: one byte per plant
//   bits 0-1 = water, bits 2-3 = light   (0 = LOW, 1 = MED, 2 = HI)
//   0xFF = nothing saved for that plant
// Extra bytes: water time hour, minute
// ---------------------------------------------------------------

type Plants = [[(WaterLevel, LightLevel); MAX_PLANTS]; ROWS];

const _: () = assert!(ROWS * MAX_PLANTS <= CELLS, "too many plants for the settings record");
const _: () = assert!(EXTRA >= 2, "settings record needs 2 extra bytes for the water time");

fn pack(plants: &Plants) -> [u8; CELLS] {
    let mut cells = [0xFF; CELLS];
    for r in 0..ROWS {
        for p in 0..MAX_PLANTS {
            let (water, light) = plants[r][p];
            let w = match water {
                WaterLevel::Low => 0,
                WaterLevel::Med => 1,
                WaterLevel::High => 2,
            };
            let l = match light {
                LightLevel::Low => 0,
                LightLevel::Med => 1,
                LightLevel::High => 2,
            };
            cells[r * MAX_PLANTS + p] = w | (l << 2);
        }
    }
    cells
}

fn unpack(cells: &[u8; CELLS], plants: &mut Plants) {
    for r in 0..ROWS {
        for p in 0..MAX_PLANTS {
            let c = cells[r * MAX_PLANTS + p];
            let water = match c & 0b11 {
                0 => WaterLevel::Low,
                1 => WaterLevel::Med,
                2 => WaterLevel::High,
                _ => continue, // 0xFF or invalid: keep the default
            };
            let light = match (c >> 2) & 0b11 {
                0 => LightLevel::Low,
                1 => LightLevel::Med,
                2 => LightLevel::High,
                _ => continue,
            };
            plants[r][p] = (water, light);
        }
    }
}

/// Save plant settings and the water time together (one flash record).
fn save_all(store: &mut Store, plants: &Plants, water_time: (u8, u8)) -> bool {
    let mut extra = [0xFF; EXTRA];
    extra[0] = water_time.0;
    extra[1] = water_time.1;
    store.save(&pack(plants), &extra)
}

// -----------------------------------------------------------------------------
// control loop for each plant
// -----------------------------------------------------------------------------
//
// Once a day at the WATER TIME (or every P_WATER_INTERVAL_S while the clock
// isn't set), go through the plants one at a time (in series).
// For each plant below its target moisture, water in stages:
//
//   stage 1: water up to  75% of target   (P_WATER_STAGES[0])
//            wait P_SOAK_S, re-measure; still below target?
//   stage 2: water up to  95% of target   (P_WATER_STAGES[1])
//            wait, re-measure; still below?
//   stage 3: water up to 105% of target
//            wait, re-measure; still below?
//   stage 4: water up to 115% of target
//            wait, re-measure; still below? -> NOT WET fault, next plant
//
// "Water up to X%" = pump the volume for the gap between current moisture and
// X% of target: gap (%) * P_ML_PER_PCT, measured with the flow meter.
// Each pump run also stops on P_PUMP_MAX_S (timeout) or no flow (fault).
// Stages that are already met are skipped.
//
// It's a state machine stepped every 100 ms, so the display, joystick
// and flow meter keep running the whole time.
// ---------------------------------------------------------------

// Plants are numbered row by row: index = row * MAX_PLANTS + plant
const PLANT_SLOTS: usize = ROWS * MAX_PLANTS;

fn row_plant(idx: usize) -> (usize, usize) {
    (idx / MAX_PLANTS, idx % MAX_PLANTS)
}

/// Sensor and valve of a plant, only if its pod is installed.
fn sensor_valve(idx: usize, hygro_ok: &[bool; SENSORS]) -> Option<(usize, usize)> {
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
fn valve_of(idx: usize) -> Option<usize> {
    let (r, p) = row_plant(idx);
    if r >= ROWS {
        return None;
    }
    VALVE_OF[r][p]
}

/// Valve of a plant that exists in the layout (used by sys check, which tests
/// the plumbing whether or not a hygrometer is plugged in).
fn wired_valve(idx: usize) -> Option<usize> {
    let (r, p) = row_plant(idx);
    if r >= ROWS || p >= PLANTS_PER_ROW[r] {
        return None;
    }
    VALVE_OF[r][p]
}

/// Next plant at or after `from` with a wired valve.
fn next_wired_valve(from: usize) -> Option<usize> {
    (from..PLANT_SLOTS).find(|&i| wired_valve(i).is_some())
}

/// Moisture % the user's Wtr setting asks for.
fn target_pct(level: WaterLevel) -> u8 {
    match level {
        WaterLevel::Low => P_TARGET_MOISTURE[0],
        WaterLevel::Med => P_TARGET_MOISTURE[1],
        WaterLevel::High => P_TARGET_MOISTURE[2],
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Water {
    Idle,
    // Compare moisture with target, pick the next stage
    Check { idx: usize, stage: usize },
    // Pump running until `goal` flow pulses have been delivered
    Pumping { idx: usize, stage: usize, start_s: u32, start_pulses: u32, goal: u32 },
    // Waiting for the water to reach the sensor
    Soaking { idx: usize, stage: usize, until_s: u32 },
    // Waiting for a fresh moisture reading
    Measuring { idx: usize, stage: usize, reads: u32 },
}

/// First installed pod at or after `from`, or Idle when the run is done.
fn water_from(from: usize, hygro_ok: &[bool; SENSORS]) -> Water {
    match (from..PLANT_SLOTS).find(|&i| sensor_valve(i, hygro_ok).is_some()) {
        Some(idx) => Water::Check { idx, stage: 0 },
        None => Water::Idle,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Fault {
    NoFlow(usize),     // pump ran but the flow meter saw nothing
    NotReached(usize), // all stages used and still below target
}

// ---------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------

/// One character per plant, rows separated by '|', e.g. "111|1-1|---".
/// Returned as a fixed buffer + length so it works without an allocator.
fn plant_map(f: impl Fn(usize) -> u8) -> ([u8; 16], usize) {
    let mut buf = [b' '; 16];
    let mut len = 0;
    for r in 0..ROWS {
        if r > 0 && len < buf.len() {
            buf[len] = b'|';
            len += 1;
        }
        for p in 0..PLANTS_PER_ROW[r] {
            if len < buf.len() {
                buf[len] = f(r * MAX_PLANTS + p);
                len += 1;
            }
        }
    }
    (buf, len)
}

/// Seconds since midnight -> (hours, minutes)
fn hh_mm(tod: u32) -> (u8, u8) {
    ((tod / 3600) as u8, ((tod / 60) % 60) as u8)
}

#[entry]
fn main() -> ! {
    if let (Some(dp), Some(cp)) = (pac::Peripherals::take(), cortex_m::peripheral::Peripherals::take())
    {
        // Set up the system clock. We want to run at 48MHz for this one.
        // NOTE: HSI is only ~1% accurate, so the software clock can drift several minutes a day.
        // The Nucleo feeds an accurate 8 MHz from the ST-LINK into HSE; switching to it fixes that.
        let mut rcc = dp.RCC.freeze(Config::hsi().sysclk(48.MHz()));
        let sysclk_hz: u32 = rcc.clocks.sysclk().raw();

        // Cycle counter for the task cycle timing
        let mut dcb = cp.DCB;
        let mut dwt = cp.DWT;
        dcb.enable_trace();
        dwt.enable_cycle_counter();

        // GPIO
        let gpioa = dp.GPIOA.split(&mut rcc);
        let gpiob = dp.GPIOB.split(&mut rcc);
        let gpioc = dp.GPIOC.split(&mut rcc);

        // Joystick inputs
        let x_pin = gpioa.pa0.into_analog();
        let y_pin = gpioa.pa1.into_analog();
        let sw = gpioa.pa10.into_pull_up_input(); // button pulls to GND when pressed

        // Hygrometer inputs + power switch (starts OFF: gate high = P-MOSFET off)
        let m0_pin = gpioa.pa4.into_analog(); // R1 P1
        let m1_pin = gpiob.pb0.into_analog(); // R1 P2
        let m2_pin = gpioc.pc1.into_analog(); // R1 P3
        let m3_pin = gpioc.pc0.into_analog(); // R2 P1
        let m4_pin = gpiob.pb1.into_analog(); // R2 P2
        let m5_pin = gpioc.pc2.into_analog(); // R2 P3
        let mut sensor_pwr = gpiob.pb3.into_push_pull_output_in_state(PinState::High);

        // ADC1, 12-bit by default (shared by joystick and hygrometers)
        let mut adc = Adc::new(dp.ADC1, true, AdcConfig::default(), &mut rcc);

        // Fault LED
        let mut led_hmi = gpiob.pb5.into_push_pull_output();
        let mut led_on = true;

        // Valves: all closed at startup. Index = valve number (see VALVE_OF).
        // Set one with valves[i].set_high() / set_low().
        let mut valves = [
            gpiob.pb4.into_push_pull_output_in_state(PinState::Low).erase(),  // 0: R1 P1
            gpiob.pb10.into_push_pull_output_in_state(PinState::Low).erase(), // 1: R1 P2
            gpioa.pa8.into_push_pull_output_in_state(PinState::Low).erase(),  // 2: R1 P3
            gpioa.pa9.into_push_pull_output_in_state(PinState::Low).erase(),  // 3: R2 P1
            gpioc.pc7.into_push_pull_output_in_state(PinState::Low).erase(),  // 4: R2 P2
            gpiob.pb6.into_push_pull_output_in_state(PinState::Low).erase(),  // 5: R2 P3
        ];

        // Motor / pump: off at startup
        let mut pump = gpioc.pc6.into_push_pull_output_in_state(PinState::Low);

        // Flow meter: interrupt on every rising edge of PC8
        let mut syscfg = dp.SYSCFG.constrain(&mut rcc);
        let mut exti = dp.EXTI;
        let mut flow_pin = gpioc.pc8; // input by default
        flow_pin.make_interrupt_source(&mut syscfg);
        flow_pin.trigger_on_edge(&mut exti, Edge::Rising);
        flow_pin.enable_interrupt(&mut exti);
        cortex_m::interrupt::free(|cs| FLOW_PIN.borrow(cs).replace(Some(flow_pin)));
        enable_flow_interrupt();

        // Create a delay abstraction based on SysTick (used by the LCD driver)
        let mut delay = cp.SYST.delay(&rcc.clocks);

        // I2C1 on the Arduino D15 (SCL) / D14 (SDA) pins
        let i2c = dp.I2C1.i2c((gpiob.pb8, gpiob.pb9), 100.kHz(), &mut rcc);
        let mut lcd = Lcd::new(i2c, LCD_ADDR, &mut delay).unwrap();

        // Plant settings: [row][plant] = (water, light). Sized for MAX_PLANTS per row,
        // only the first PLANTS_PER_ROW[row] entries are used.
        // Loaded from flash if saved before, otherwise every plant starts at MED/MED.
        let mut plants: Plants = [[(WaterLevel::Med, LightLevel::Med); MAX_PLANTS]; ROWS];
        let mut water_time: (u8, u8) = DEFAULT_WATER_TIME; // daily watering time (HH, MM)
        let (mut store, saved) = Store::open(dp.FLASH, MAX_PLANTS as u8);
        if let Some((cells, extra)) = saved {
            unpack(&cells, &mut plants);
            if extra[0] < 24 && extra[1] < 60 {
                water_time = (extra[0], extra[1]); // older records have 0xFF here: keep default
            }
        }
        let mut save_failed = false;

        // UI state
        let mut row: usize = 0;
        let mut plant: usize = 0;
        let mut mode = Mode::Browse;
        let mut last_dir = Dir::Center;
        let mut hold_ticks: u16 = 0; // how long the stick has been held in one direction
        let mut press_ticks: u32 = 0; // how long the button has been held
        let mut cooldown: u8 = 0;
        let mut redraw = true;
        let mut drawing = false; // redraw in progress: line 1 done, line 2 on the next tick
        let mut menu_msg: Option<&'static str> = None; // e.g. "BUSY: WATERING"
        let long_press_ticks: u32 = (P_LONG_PRESS_MS / TICK_MS).max(1);

        // Sensor state
        let mut moisture: [u8; SENSORS] = [0; SENSORS]; // percent, per sensor
        let mut hygro_ok: [bool; SENSORS] = [false; SENSORS]; // plugged in, from the last reading
        let mut moisture_reads: u32 = 0; // completed readings, so watering can wait for a fresh one
        let mut seconds: u32 = 0; // uptime, counted by the 1000 ms task
        let mut next_moisture_s: u32 = 0; // read immediately at startup
        let mut last_total_pulses: u32 = 0;
        let mut flow_lpm: f32 = 0.0;
        let mut total_liters: f32 = 0.0;
        let mut showing_flow = false;
        let mut sensor_on_at: Option<u32> = None; // tick when sensors were powered

        // Clock: time of day = (seconds + clock_offset) % 86400, once set from the menu
        let mut clock_set = false;
        let mut clock_offset: u32 = 0;
        let mut last_water_day: Option<u32> = None; // day number of the last daily run

        // Watering state
        let mut water = Water::Idle;
        let mut water_run_due = false;
        let mut next_water_s: u32 = 0; // fallback schedule while the clock isn't set
        let mut water_fault: Option<Fault> = None;

        // Sys check results, per plant slot: b'1' ok, b'X' fail, b'-' no valve
        let mut sys_result: [u8; PLANT_SLOTS] = [b'-'; PLANT_SLOTS];
        let mut sys_leak = false;

        // -----------------------------------------------------------------------------------------------------
        // Setting the Task Cycles here
        // Chosed 10ms to be easy to calculate to, good enough for the sensors and feeback loop
        // Will need to add a constant multiplier to equal to seconds for easy calculations
        // Can set as either a parameter to a constant, but don't expect it to ever change, so Const is better
        // -----------------------------------------------------------------------------------------------------

        const TICK_MS: u32 = 10;
        let tick = sysclk_hz / 1000 * TICK_MS;   // cycles per tick
        let mut next = DWT::cycle_count().wrapping_add(tick);
        let mut n: u32 = 0;
        let mut overruns: u32 = 0;
        let mut skip_overrun = false; // a flash save ran this tick (can take a sector erase)

        // -----------------------------------------------------------------------------------------------------
        // -----------------------------------------------------------------------------------------------------

        loop
        {
            // Wait for the next 10 ms tick (wrap-safe)
            while (DWT::cycle_count().wrapping_sub(next) as i32) < 0 {}
            next = next.wrapping_add(tick);
            n = n.wrapping_add(1);

            let tod = (seconds % SECONDS_PER_DAY + clock_offset) % SECONDS_PER_DAY; // time of day, s

        // -----------------------------------------------------------------------------
        // 10ms tasks
        // -----------------------------------------------------------------------------

            // -----------------------
            // Button: click on release (< 1 s), long press fires at 1 s while held
            // -----------------------
            let pressed = sw.is_low(); // active-low
            let mut click = false;
            let mut long_press = false;
            if pressed {
                press_ticks = press_ticks.saturating_add(1);
                if press_ticks == long_press_ticks && cooldown == 0 {
                    long_press = true;
                    cooldown = P_BUTTON_COOLDOWN;
                }
            } else {
                // >= 2 ticks filters contact bounce; long presses already fired
                if press_ticks >= 2 && press_ticks < long_press_ticks && cooldown == 0 {
                    click = true;
                    cooldown = P_BUTTON_COOLDOWN;
                }
                press_ticks = 0;
            }
            if cooldown > 0 && !pressed {
                cooldown -= 1;
            }

            if long_press {
                mode = match mode {
                    Mode::Browse | Mode::Edit(_) => Mode::Menu(0),
                    Mode::SysCheck { .. } => {
                        // Abort: everything off
                        pump.set_low();
                        for v in valves.iter_mut() {
                            v.set_low();
                        }
                        led_hmi.set_low();
                        Mode::Browse
                    }
                    _ => Mode::Browse, // any menu screen: back to browsing
                };
                menu_msg = None;
                redraw = true;
            }

            if click {
                mode = match mode {
                    // Only installed pods can be edited
                    Mode::Browse if plant_pod_installed(row, plant, &hygro_ok) => Mode::Edit(Field::Water),
                    Mode::Browse => Mode::Browse,
                    Mode::Edit(_) => Mode::Browse,

                    Mode::Menu(MENU_RELEARN) => {
                        // Read all hygrometers right now
                        if sensor_on_at.is_none() {
                            sensor_pwr.set_low();
                            sensor_on_at = Some(n);
                        }
                        Mode::Relearn { reads: moisture_reads }
                    }
                    Mode::Menu(MENU_SYSCHECK) => {
                        if water == Water::Idle {
                            sys_result = [b'-'; PLANT_SLOTS];
                            sys_leak = false;
                            led_hmi.set_high();
                            Mode::SysCheck {
                                step: Sys::Led,
                                start_n: n,
                                start_pulses: FLOW_PULSES.load(Ordering::Relaxed),
                            }
                        } else {
                            menu_msg = Some("BUSY: WATERING");
                            Mode::Menu(MENU_SYSCHECK)
                        }
                    }
                    Mode::Menu(MENU_CLOCK) => {
                        let (hh, mm) = if clock_set { hh_mm(tod) } else { (12, 0) };
                        Mode::SetTime { what: TimeSet::Clock, hh, mm, field: TimeField::Hour }
                    }
                    Mode::Menu(MENU_WATER_TIME) => Mode::SetTime {
                        what: TimeSet::Water,
                        hh: water_time.0,
                        mm: water_time.1,
                        field: TimeField::Hour,
                    },
                    Mode::Menu(_) => Mode::Browse, // EXIT

                    Mode::Relearn { reads } => Mode::Relearn { reads }, // still reading
                    Mode::RelearnResult => Mode::Menu(MENU_RELEARN),
                    Mode::SysCheck { step, start_n, start_pulses } => {
                        Mode::SysCheck { step, start_n, start_pulses } // hold to abort
                    }
                    Mode::SysResult => Mode::Menu(MENU_SYSCHECK),

                    Mode::SetTime { what: TimeSet::Clock, hh, mm, .. } => {
                        // Clock now reads hh:mm:00
                        let set = hh as u32 * 3600 + mm as u32 * 60;
                        clock_offset = (set + SECONDS_PER_DAY - seconds % SECONDS_PER_DAY) % SECONDS_PER_DAY;
                        clock_set = true;
                        last_water_day = None;
                        Mode::Menu(MENU_CLOCK)
                    }
                    Mode::SetTime { what: TimeSet::Water, hh, mm, .. } => {
                        water_time = (hh, mm);
                        save_failed = !save_all(&mut store, &plants, water_time);
                        skip_overrun = true;
                        Mode::Menu(MENU_WATER_TIME)
                    }
                };
                redraw = true;
            }


            // -----------------------
            // Joystick stuff here:
            // -----------------------
            // converting the analog inputs to digital values
            let x: u16 = adc.convert(&x_pin, SampleTime::Cycles_480);
            let y: u16 = adc.convert(&y_pin, SampleTime::Cycles_480);
            let dir = read_dir(x, y);

            // One step each time the stick leaves the center. In the time editor,
            // holding up/down auto-repeats (after 0.5 s, every 150 ms).
            if dir != Dir::Center && dir == last_dir {
                hold_ticks = hold_ticks.saturating_add(1);
            } else {
                hold_ticks = 0;
            }
            let edge = dir != last_dir && last_dir == Dir::Center;
            let repeat = matches!(mode, Mode::SetTime { .. })
                && matches!(dir, Dir::Up | Dir::Down)
                && hold_ticks >= 50
                && (hold_ticks - 50) % 15 == 0;
            last_dir = dir;

            if edge || repeat {
                menu_msg = None;
                match mode {
                    Mode::Browse => {
                        match dir {
                            Dir::Up => row = (row + ROWS - 1) % ROWS,
                            Dir::Down => row = (row + 1) % ROWS,
                            Dir::Left => {
                                let count = PLANTS_PER_ROW[row];
                                plant = (plant + count - 1) % count;
                            }
                            Dir::Right => plant = (plant + 1) % PLANTS_PER_ROW[row],
                            Dir::Center => {}
                        }
                        // Moving to a shorter row: stay on its last plant
                        plant = plant.min(PLANTS_PER_ROW[row] - 1);
                    }
                    Mode::Edit(field) => {
                        let before = plants[row][plant];
                        let entry = &mut plants[row][plant];
                        match (dir, field) {
                            (Dir::Left | Dir::Right, _) => mode = Mode::Edit(field.other()),
                            (Dir::Up, Field::Water) => entry.0 = entry.0.up(),
                            (Dir::Down, Field::Water) => entry.0 = entry.0.down(),
                            (Dir::Up, Field::Light) => entry.1 = entry.1.up(),
                            (Dir::Down, Field::Light) => entry.1 = entry.1.down(),
                            (Dir::Center, _) => {}
                        }
                        // Save every change (pushing past HI/LOW changes nothing, so no save)
                        if plants[row][plant] != before {
                            save_failed = !save_all(&mut store, &plants, water_time);
                            skip_overrun = true; // rare 1-2 s sector erase is expected, not a fault
                        }
                    }
                    Mode::Menu(item) => {
                        mode = match dir {
                            Dir::Up => Mode::Menu((item + MENU.len() - 1) % MENU.len()),
                            Dir::Down => Mode::Menu((item + 1) % MENU.len()),
                            Dir::Left => Mode::Browse,
                            _ => mode,
                        };
                    }
                    Mode::RelearnResult if dir == Dir::Left => mode = Mode::Menu(MENU_RELEARN),
                    Mode::SysResult if dir == Dir::Left => mode = Mode::Menu(MENU_SYSCHECK),
                    Mode::SetTime { what, hh, mm, field } => {
                        let (hh, mm, field) = match (dir, field) {
                            (Dir::Left | Dir::Right, TimeField::Hour) => (hh, mm, TimeField::Minute),
                            (Dir::Left | Dir::Right, TimeField::Minute) => (hh, mm, TimeField::Hour),
                            (Dir::Up, TimeField::Hour) => ((hh + 1) % 24, mm, field),
                            (Dir::Down, TimeField::Hour) => ((hh + 23) % 24, mm, field),
                            (Dir::Up, TimeField::Minute) => (hh, (mm + 1) % 60, field),
                            (Dir::Down, TimeField::Minute) => (hh, (mm + 59) % 60, field),
                            (Dir::Center, _) => (hh, mm, field),
                        };
                        mode = Mode::SetTime { what, hh, mm, field };
                    }
                    _ => {} // Relearn / SysCheck in progress: stick does nothing
                }
                redraw = true;
            }

            // -----------------------
            // Sys check sequence (10 ms resolution)
            // -----------------------
            if let Mode::SysCheck { step, start_n, start_pulses } = mode {
                let elapsed_ms = n.wrapping_sub(start_n) * TICK_MS;
                let now_pulses = FLOW_PULSES.load(Ordering::Relaxed);
                let step_lpm = now_pulses.wrapping_sub(start_pulses) as f32 / P_FLOW_PULSES_PER_L
                    * 60_000.0
                    / P_SYSCHK_STEP_MS as f32;
                let flow_ok = step_lpm >= P_SYSCHK_MIN_LPM && step_lpm <= P_SYSCHK_MAX_LPM;

                // Moving to the next valve: the next valve opens BEFORE the previous
                // one closes, so the pump always has somewhere to push.
                // After the last valve: pump off first, then close.
                let next_step = |next_idx: Option<usize>| match next_idx {
                    Some(i) => Mode::SysCheck { step: Sys::Valve(i), start_n: n, start_pulses: now_pulses },
                    None => Mode::SysResult,
                };

                match step {
                    Sys::Led if elapsed_ms >= P_SYSCHK_LED_MS => {
                        led_hmi.set_low();
                        pump.set_high(); // all valves still closed
                        mode = Mode::SysCheck { step: Sys::PumpOnly, start_n: n, start_pulses: now_pulses };
                        redraw = true;
                    }
                    Sys::PumpOnly if elapsed_ms >= P_SYSCHK_STEP_MS => {
                        sys_leak = step_lpm >= P_SYSCHK_MIN_LPM; // flow with every valve shut
                        let next_idx = next_wired_valve(0);
                        match next_idx.and_then(wired_valve) {
                            Some(v) => valves[v].set_high(),
                            None => pump.set_low(),
                        }
                        mode = next_step(next_idx);
                        redraw = true;
                    }
                    Sys::Valve(idx) if elapsed_ms >= P_SYSCHK_STEP_MS => {
                        sys_result[idx] = if flow_ok { b'1' } else { b'X' };
                        let next_idx = next_wired_valve(idx + 1);
                        match next_idx.and_then(wired_valve) {
                            Some(v) => valves[v].set_high(),
                            None => pump.set_low(),
                        }
                        if let Some(v) = wired_valve(idx) {
                            valves[v].set_low();
                        }
                        mode = next_step(next_idx);
                        redraw = true;
                    }
                    _ => {}
                }
            }


        // -----------------------------------------------------------------------------
        // 100ms tasks
        // tick 3: moisture read + display line 1
        // tick 4: display line 2 + cursor   (split so each tick stays inside 10 ms)
        // tick 5: watering control loop
        // -----------------------------------------------------------------------------

            if n % 10 == 3
            {

                // Sensor: Read once the warm-up has passed
                if let Some(start) = sensor_on_at {
                    if n.wrapping_sub(start) * TICK_MS >= P_SENSOR_WARMUP_MS {
                        let raw: [u16; SENSORS] = [
                            read_avg!(adc, m0_pin),
                            read_avg!(adc, m1_pin),
                            read_avg!(adc, m2_pin),
                            read_avg!(adc, m3_pin),
                            read_avg!(adc, m4_pin),
                            read_avg!(adc, m5_pin),
                        ];
                        sensor_pwr.set_high(); // off again

                        // Which hygrometers are plugged in, and their moisture
                        for i in 0..SENSORS {
                            hygro_ok[i] = hygro_detected(raw[i]);
                        }
                        // Only sensors with calibration in params.dcm
                        for i in 0..CALIBRATED {
                            moisture[i] = moisture_pct(raw[i], P_MOISTURE_DRY[i], P_MOISTURE_WET[i]);
                        }
                        moisture_reads = moisture_reads.wrapping_add(1);
                        next_moisture_s = seconds + P_MOISTURE_PERIOD_S;
                        sensor_on_at = None;
                        redraw = true;

                        // Pod unplugged while editing it: back to browsing
                        if let Mode::Edit(_) = mode {
                            if !plant_pod_installed(row, plant, &hygro_ok) {
                                mode = Mode::Browse;
                            }
                        }
                        // Relearn finished
                        if let Mode::Relearn { reads } = mode {
                            if moisture_reads != reads {
                                mode = Mode::RelearnResult;
                            }
                        }
                    }
                }


                // Display line 1
                drawing = redraw;
                redraw = false;
                if drawing {
                    match mode {
                        Mode::Menu(_) => {
                            if clock_set {
                                let (h, m) = hh_mm(tod);
                                let _ = lcd.write_line(0, format_args!("MENU       {:02}:{:02}", h, m), &mut delay);
                            } else {
                                let _ = lcd.write_line(0, format_args!("MENU       --:--"), &mut delay);
                            }
                        }
                        Mode::Relearn { .. } => {
                            let _ = lcd.write_line(0, format_args!("RELEARN..."), &mut delay);
                        }
                        Mode::RelearnResult => {
                            let found = (0..PLANT_SLOTS)
                                .filter(|&i| {
                                    let (r, p) = row_plant(i);
                                    plant_pod_installed(r, p, &hygro_ok)
                                })
                                .count();
                            let _ = lcd.write_line(0, format_args!("PODS FOUND: {}", found), &mut delay);
                        }
                        Mode::SysCheck { step, .. } => match step {
                            Sys::Led => {
                                let _ = lcd.write_line(0, format_args!("SYS CHECK: LED"), &mut delay);
                            }
                            Sys::PumpOnly => {
                                let _ = lcd.write_line(0, format_args!("SYS CHECK: PUMP"), &mut delay);
                            }
                            Sys::Valve(idx) => {
                                let (r, p) = row_plant(idx);
                                let _ = lcd.write_line(0, format_args!("SYS CHK: R{} P{}", r + 1, p + 1), &mut delay);
                            }
                        },
                        Mode::SysResult => {
                            let fails = sys_result.iter().filter(|&&c| c == b'X').count();
                            if sys_leak {
                                let _ = lcd.write_line(0, format_args!("LEAK? PUMP ONLY"), &mut delay);
                            } else if fails > 0 {
                                let _ = lcd.write_line(0, format_args!("SYS FAIL: {}", fails), &mut delay);
                            } else {
                                let _ = lcd.write_line(0, format_args!("SYS CHECK OK"), &mut delay);
                            }
                        }
                        Mode::SetTime { what: TimeSet::Clock, .. } => {
                            let _ = lcd.write_line(0, format_args!("SET CLOCK"), &mut delay);
                        }
                        Mode::SetTime { what: TimeSet::Water, .. } => {
                            let _ = lcd.write_line(0, format_args!("SET WATER TIME"), &mut delay);
                        }

                        // Browse / Edit, priority: save error > watering fault > watering status
                        //                          > flow > header + this plant's moisture
                        Mode::Browse | Mode::Edit(_) => {
                            if save_failed {
                                let _ = lcd.write_line(0, format_args!("SAVE FAILED"), &mut delay);
                            } else if let Some(f) = water_fault {
                                let (text, idx) = match f {
                                    Fault::NoFlow(i) => ("NO FLOW", i),
                                    Fault::NotReached(i) => ("NOT WET", i),
                                };
                                let (r, p) = row_plant(idx);
                                let _ = lcd.write_line(0, format_args!("{} R{} P{}", text, r + 1, p + 1), &mut delay);
                            } else if let Water::Pumping { idx, stage, start_pulses, goal, .. } = water {
                                let (r, p) = row_plant(idx);
                                let to_ml = |pulses: u32| (pulses as f32 * 1000.0 / P_FLOW_PULSES_PER_L) as u32;
                                let delivered = FLOW_PULSES.load(Ordering::Relaxed).wrapping_sub(start_pulses);
                                let _ = lcd.write_line(
                                    0,
                                    format_args!("R{}P{} S{} {}/{}mL", r + 1, p + 1, stage + 1, to_ml(delivered), to_ml(goal)),
                                    &mut delay,
                                );
                            } else if let Water::Soaking { idx, stage, until_s } = water {
                                let (r, p) = row_plant(idx);
                                let _ = lcd.write_line(
                                    0,
                                    format_args!("R{}P{} S{} SOAK {}s", r + 1, p + 1, stage + 1, until_s.saturating_sub(seconds)),
                                    &mut delay,
                                );
                            } else if let Water::Check { idx, .. } | Water::Measuring { idx, .. } = water {
                                let (r, p) = row_plant(idx);
                                let _ = lcd.write_line(0, format_args!("R{}P{} CHECK", r + 1, p + 1), &mut delay);
                            } else if showing_flow {
                                let _ = lcd.write_line(
                                    0,
                                    format_args!("{:>5.2}L/m {:>5.1}L", flow_lpm, total_liters),
                                    &mut delay,
                                );
                            } else if let (true, Some(s)) =
                                (plant_pod_installed(row, plant, &hygro_ok), SENSOR_OF[row][plant])
                            {
                                let _ = lcd.write_line(
                                    0,
                                    format_args!("R#|P#|Wtr|Lt{:>3}%", moisture[s]),
                                    &mut delay,
                                );
                            } else {
                                // Pod not installed
                                let _ = lcd.write_line(0, format_args!("R#|P#|Wtr|Lt  --"), &mut delay);
                            }
                        }
                    }
                }
            }

            if n % 10 == 4 && drawing
            {
                // Display line 2 + cursor
                let mut cursor: Option<u8> = None; // blinking cursor column on line 2
                match mode {
                    Mode::Menu(item) => {
                        let text = menu_msg.unwrap_or(MENU[item]);
                        let _ = lcd.write_line(1, format_args!("> {}", text), &mut delay);
                    }
                    Mode::Relearn { .. } => {
                        let _ = lcd.write_line(1, format_args!("READING SENSORS"), &mut delay);
                    }
                    Mode::RelearnResult => {
                        // 1 = pod installed, - = not, per plant, rows split by '|'
                        let (buf, len) = plant_map(|i| {
                            let (r, p) = row_plant(i);
                            if plant_pod_installed(r, p, &hygro_ok) { b'1' } else { b'-' }
                        });
                        let map = core::str::from_utf8(&buf[..len]).unwrap_or("?");
                        let _ = lcd.write_line(1, format_args!("{}", map), &mut delay);
                    }
                    Mode::SysCheck { .. } => {
                        let _ = lcd.write_line(1, format_args!("HOLD BTN: ABORT"), &mut delay);
                    }
                    Mode::SysResult => {
                        // 1 = valve flow ok, X = failed, - = no valve
                        let (buf, len) = plant_map(|i| sys_result[i]);
                        let map = core::str::from_utf8(&buf[..len]).unwrap_or("?");
                        let _ = lcd.write_line(1, format_args!("{}", map), &mut delay);
                    }
                    Mode::SetTime { hh, mm, field, .. } => {
                        let _ = lcd.write_line(1, format_args!("     {:02}:{:02}", hh, mm), &mut delay);
                        cursor = Some(match field {
                            TimeField::Hour => HOUR_COL,
                            TimeField::Minute => MIN_COL,
                        });
                    }
                    Mode::Browse | Mode::Edit(_) => {
                        // Selected plant ("--" for a pod that isn't installed)
                        let (water_set, light_set) = plants[row][plant];
                        let (wtr, lt) = if plant_pod_installed(row, plant, &hygro_ok) {
                            (water_set.as_str(), light_set.as_str())
                        } else {
                            ("--", "--")
                        };
                        let marker = if mode == Mode::Browse { ' ' } else { '*' };
                        let _ = lcd.write_line(
                            1,
                            format_args!("R{} P{} {:<3} {:<3}  {}", row + 1, plant + 1, wtr, lt, marker),
                            &mut delay,
                        );
                        if let Mode::Edit(field) = mode {
                            cursor = Some(match field {
                                Field::Water => WATER_COL,
                                Field::Light => LIGHT_COL,
                            });
                        }
                    }
                }

                // Blinking cursor on the field being edited
                match cursor {
                    Some(col) => {
                        let _ = lcd.set_cursor(col, 1, &mut delay);
                        let _ = lcd.cursor_blink(true, &mut delay);
                    }
                    None => {
                        let _ = lcd.cursor_blink(false, &mut delay);
                    }
                }
                drawing = false;
            }

            if n % 10 == 5
            {
                // -------------------------------------------------
                // Watering control loop (one step per 100 ms)
                // -------------------------------------------------
                let sys_busy = matches!(mode, Mode::SysCheck { .. });
                water = match water {
                    Water::Idle => {
                        if water_run_due && moisture_reads > 0 && !sys_busy {
                            water_run_due = false;
                            water_fault = None; // a new run clears the last run's fault
                            water_from(0, &hygro_ok)
                        } else {
                            Water::Idle
                        }
                    }

                    Water::Check { idx, stage } => {
                        let (r, p) = row_plant(idx);
                        match sensor_valve(idx, &hygro_ok) {
                            Some((s, v)) => {
                                let current = moisture[s] as u32;
                                let target = target_pct(plants[r][p].0) as u32;
                                if current >= target {
                                    water_from(idx + 1, &hygro_ok) // wet enough: next plant
                                } else if stage >= P_WATER_STAGES.len() {
                                    water_fault = Some(Fault::NotReached(idx)); // all stages used
                                    water_from(idx + 1, &hygro_ok)
                                } else {
                                    // Stage level, e.g. 75% of a 50% target = 37%
                                    let stage_level = target * P_WATER_STAGES[stage] as u32 / 100;
                                    if current >= stage_level {
                                        Water::Check { idx, stage: stage + 1 } // stage already met
                                    } else {
                                        // Volume for the gap, converted to flow meter pulses
                                        let gap_pct = (stage_level - current) as f32;
                                        let ml = gap_pct * P_ML_PER_PCT;
                                        let goal = ((ml * P_FLOW_PULSES_PER_L / 1000.0) as u32).max(1);

                                        valves[v].set_high(); // open the valve first,
                                        pump.set_high(); // so the pump never pushes against closed valves
                                        Water::Pumping {
                                            idx,
                                            stage,
                                            start_s: seconds,
                                            start_pulses: FLOW_PULSES.load(Ordering::Relaxed),
                                            goal,
                                        }
                                    }
                                }
                            }
                            None => water_from(idx + 1, &hygro_ok), // pod not installed: skip
                        }
                    }

                    Water::Pumping { idx, stage, start_s, start_pulses, goal } => {
                        let delivered = FLOW_PULSES.load(Ordering::Relaxed).wrapping_sub(start_pulses);
                        let run_s = seconds - start_s;
                        let no_flow = run_s >= P_FLOW_CHECK_S && flow_lpm < FLOW_SHOW_MIN_LPM;
                        let (r, p) = row_plant(idx);
                        let removed = !plant_pod_installed(r, p, &hygro_ok);

                        if delivered >= goal || no_flow || removed || run_s >= P_PUMP_MAX_S {
                            pump.set_low(); // pump off first, then close the valve
                            if let Some(v) = valve_of(idx) {
                                valves[v].set_low(); // always closes, even if the sensor vanished
                            }
                            if no_flow {
                                water_fault = Some(Fault::NoFlow(idx));
                                water_from(idx + 1, &hygro_ok)
                            } else if removed {
                                water_from(idx + 1, &hygro_ok) // pod unplugged mid-watering
                            } else {
                                Water::Soaking { idx, stage, until_s: seconds + P_SOAK_S }
                            }
                        } else {
                            water
                        }
                    }

                    Water::Soaking { idx, stage, until_s } => {
                        if seconds >= until_s {
                            next_moisture_s = seconds; // ask for a fresh reading now
                            Water::Measuring { idx, stage, reads: moisture_reads }
                        } else {
                            water
                        }
                    }

                    Water::Measuring { idx, stage, reads } => {
                        if moisture_reads != reads {
                            Water::Check { idx, stage: stage + 1 } // fresh reading: next stage
                        } else {
                            water
                        }
                    }
                };

                if let Water::Pumping { .. } = water {
                    redraw = true; // live mL count while pumping
                }


            }

        // -----------------------------------------------------------------------------
        // 1000ms tasks
        // -----------------------------------------------------------------------------

            if n % 100 == 7
            {
                let total = FLOW_PULSES.load(Ordering::Relaxed);
                let pulses = total.wrapping_sub(last_total_pulses);
                last_total_pulses = total;
                seconds += 1;

                flow_lpm = pulses as f32 / P_FLOW_PULSES_PER_L * 60.0; // 1 s window
                total_liters = total as f32 / P_FLOW_PULSES_PER_L;

                let flowing = flow_lpm > FLOW_SHOW_MIN_LPM;
                if flowing || flowing != showing_flow || water != Water::Idle {
                    redraw = true;
                }
                showing_flow = flowing;
                if matches!(mode, Mode::Menu(_)) {
                    redraw = true; // keep the menu clock current
                }

                // Watering schedule
                if clock_set {
                    // Daily, in the minute of the WATER TIME
                    let t = seconds as u64 + clock_offset as u64;
                    let now_tod = (t % SECONDS_PER_DAY as u64) as u32;
                    let day = (t / SECONDS_PER_DAY as u64) as u32;
                    let start = water_time.0 as u32 * 3600 + water_time.1 as u32 * 60;
                    if now_tod >= start && now_tod < start + 60 && last_water_day != Some(day) {
                        last_water_day = Some(day);
                        water_run_due = true;
                    }
                } else if seconds >= next_water_s && moisture_reads > 0 {
                    // Clock not set (e.g. after a power loss): fall back to a fixed interval
                    next_water_s = seconds + P_WATER_INTERVAL_S;
                    water_run_due = true;
                }

                // Sensor: Start a reading when due
                if seconds >= next_moisture_s && sensor_on_at.is_none() {
                    sensor_pwr.set_low(); // P-MOSFET on
                    sensor_on_at = Some(n);
                }

                // -------------------------------------------------
                // FAULT logic
                // -------------------------------------------------
                let fault = overruns > 0 || water_fault.is_some() || save_failed;

                // For not creating simple 1s blinking inteval if theres an issue - can add different sequences later
                // (sys check drives the LED itself)
                if !matches!(mode, Mode::SysCheck { .. }) {
                    if fault == true
                    {

                        if led_on == false
                        {
                            led_hmi.set_low();
                            led_on = true;
                        }
                        else if led_on == true {
                            led_hmi.set_high();
                            led_on = false;
                        }

                    } else {
                        led_hmi.set_low();
                    }
                }

            }


            // Overrun check: this cycle ran past its 10 ms slot
            if skip_overrun {
                // After a flash save, restart the schedule from now instead of
                // racing through the ticks a sector erase may have skipped
                next = DWT::cycle_count().wrapping_add(tick);
                skip_overrun = false;
            } else if (DWT::cycle_count().wrapping_sub(next) as i32) >= 0 {
                overruns += 1;
            }

        }

    }

    loop {}

    // Other functions:
    //- Show plant active sensor readings
    //- Sensor calibration options

}
