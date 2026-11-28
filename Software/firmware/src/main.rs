

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
  - 3.3V input
LED's
  - LED HMI (red, on the case) connected to PB5 / Arduino D4
      fast flash (2 Hz) = severity 1-2 fault (e.g. NO FLOW, LOW WATER),
      slow blink (0.5 Hz) = severity 3 fault (e.g. NOT WET), off otherwise
LCD - I2C 16x2
  - SCL connected to PB8 / Arduino D15
  - SDA connected to PB9 / Arduino D14
  - 5V input
----------------------------------------------------------------------

Control Loop:

Wall layout: 2 rows, 3 plants per row (6 plant pods)

Moisture sensors (capacitive v1.2/v2.0, AOUT pin, VCC straight to 5V, always powered:
  no exposed metal, so no corrosion and no need to switch them off between readings)
  Each AOUT input needs a 100k pull-down resistor to GND (board side of the connector),
  so an unplugged sensor reads ~0 instead of floating (used by plant_pod_installed).
  The ADC pins are NOT 5V tolerant: AOUT must stay below 3.3V. Boards with the
  onboard 3.3V regulator (662K) are fine; otherwise use a 10k/20k divider per
  input (its 20k to GND then also acts as the pull-down).
  - Sensor 0 (Row 1 Plant 1) connected to PA4 / Arduino A2
  - Sensor 1 (Row 1 Plant 2) connected to PB0 / Arduino A3
  - Sensor 2 (Row 1 Plant 3) connected to PC1 / Arduino A4
  - Sensor 3 (Row 2 Plant 1) connected to PC0 / Arduino A5
  - Sensor 4 (Row 2 Plant 2) connected to PB1 (Morpho CN10)
  - Sensor 5 (Row 2 Plant 3) connected to PC2 (Morpho CN7)
  - Spare analog pins: PC3, PC4, PC5, PA5.
    PA2/PA3 are the ST-LINK USB serial, keep them free.
Flow meter - hall effect, 5V powered
  - Signal connected to PC8 (10kOhm pull up connected from Analog to 5V)
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
Grow lights, one per pod (GPIO out, HIGH = on). On for the pod's light hours
per day (0-24, set in PROFILE), starting at P_LIGHT_START_H.
  - Light Row 1 Plant 1 connected to PA6 / Arduino D12 w/ MOSFET
  - Light Row 1 Plant 2 connected to PA7 / Arduino D11 w/ MOSFET
  - Light Row 1 Plant 3 connected to PB12 (Morpho CN10) w/ MOSFET
  - Light Row 2 Plant 1 connected to PB13 (Morpho CN10) w/ MOSFET
  - Light Row 2 Plant 2 connected to PB14 (Morpho CN10) w/ MOSFET
  - Light Row 2 Plant 3 connected to PB15 (Morpho CN10) w/ MOSFET
  - 5V input (logic-level N-MOSFET low side, 100k gate pull-down)
Water level sensor (limit / float switch in the reservoir)
  - One switch terminal to PB3 / Arduino D3, the other to GND.
    PB3 uses the internal pull-up, so no resistor is needed.
  - Mount it so the switch is CLOSED while there is enough water.
    That way a broken or unplugged wire also shows the LOW WATER fault.
  - LOW WATER is a fault (severity 2) but watering keeps running.
    Clears by itself after a refill.
----------------------------------------------------------------------

Screens (all text is built and unit-tested in plant_logic/src/display.rs):

  MAIN         "R# P# ACT TGT SM"   header (every 2 s it shows the worst active
               "R1 P2 42% 50% OK"   fault instead, if there is one)
               ACT = moisture now, TGT = target, SM = state:
                 OK at target   LO below target (watered at the water time)
                 WT watering    SK soaking in   LN pot learning
                 FL has a fault OF target OFF   -- pod not installed
               up/down (or left/right) = scroll through the plants,
               click = PROFILE for this plant, hold 1 s = MENU

  MENU (up/down to pick, click to select, left to go back, hold 1 s = main screen):
    PROFILE    pick a plant, then its water target (0-90 %, OFF) and light
               hours (0-24 h, OFF; new plants start at 12 h). Left/right moves
               between plant / water / light, up/down changes it. Saved at once.
    RELEARN    re-reads all sensors and shows which pods are found (and learns the
               joystick center with P_FS_JOY_RESET = 1). Click then picks a plant
               to learn its pot size: water is given in steps until the soil
               reading rises, giving mL per % (see plant_logic/src/learn.rs).
               Plants never learned use P_POT_ML_PER_PCT (a large pot).
    CALIBRATE  live sensor voltages; right moves to LOW (dry) / HI (wet), click
               stores the current reading
    TIME       CLOCK (time of day, lost on power loss) and WATER (daily watering
               time, default 09:00, saved)
    FAULTS     every fault: active ones first, most severe first, then the
               inactive ones (severities in plant_logic/src/faults.rs)
    MANUAL     every input (live raw value) and output (click toggles ON/OFF).
               Automatic watering, lights and the LED pause while it is open;
               everything is switched off when leaving. The pump turns itself
               off after P_MANUAL_PUMP_MAX_S.
    SYS CHECK  1 s per step: LED, pump alone (leak check), each valve with flow
               check, then each grow light on its own. Hold the button to abort.

Which plant each sensor and valve belongs to is set in plant_logic/src/layout.rs.
Tuning values (thresholds, calibration, timing) come from params.dcm.

All settings are saved to flash on every change (see settings.rs);
memory.x must have FLASH LENGTH = 384K so code stays out of the settings sector.
*/

#![deny(unsafe_code)]
#![allow(clippy::empty_loop)]
#![no_main]
#![no_std]

mod lcd;
//Allow plant settings to be saved
mod settings;
// Signal trace out of the ST-LINK USB serial port
mod trace_out;


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
// Everything hardware-independent (layout, calibration, PID, settings format,
// screens, faults, pot learning, ...) lives in the plant_logic crate, where it
// is unit-tested on the PC.
use plant_logic::learn::Phase;
use plant_logic::params::*;
use plant_logic::*;
use plant_logic::trace::{idx_code, Line, HEADER_EVERY_MS, LINE_MAX};
use crate::settings::Store;

// LCD constants
const LCD_ADDR: u8 = 0x27;

// LCD columns of the hour and minute in the time editor ("     07:30")
const HOUR_COL: u8 = 5;
const MIN_COL: u8 = 8;

// Flow below this (L/min) counts as "no flow" (pump fault check)
// PARAMETER!! (move when .dcm integrated)
const FLOW_SHOW_MIN_LPM: f32 = 0.05;

// An OVERRUN fault stays active this long after the last late tick
const OVERRUN_FAULT_S: u32 = 60;

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

// Write one LCD line (errors are ignored: a glitch is redrawn on the next change)
macro_rules! show {
    ($lcd:expr, $delay:expr, $row:expr, $text:expr) => {{
        let _ = $lcd.write_line($row, format_args!("{}", $text), &mut $delay);
    }};
}

// Joystick timing used by RELEARN (direction decoding is in plant_logic::joystick)
const JOY_SETTLE_TICKS: u32 = 100; // ignore the first 300 ms after the click (thumb leaving the stick)
const JOY_MIN_SAMPLES: u32 = 100; // at least 0.5 s of samples to average

// Function switch P_FS_JOY_RESET to enabl;e or diable the relearning funct.
//   1 = ON, 0 = OFF, uses the 2048.
const FS_JOY_RESET: bool = P_FS_JOY_RESET != 0;
const _: () = assert!(P_FS_JOY_RESET <= 1, "P_FS_JOY_RESET must be 0 (off) or 1 (on)");

// ---------------------------------------------------------------
// Menu / screens
// ---------------------------------------------------------------

const MENU: [&str; 8] = ["PROFILE", "RELEARN", "CALIBRATE", "TIME", "FAULTS", "MANUAL", "SYS CHECK", "EXIT"];
const MENU_PROFILE: usize = 0;
const MENU_RELEARN: usize = 1;
const MENU_CALIBRATE: usize = 2;
const MENU_TIME: usize = 3;
const MENU_FAULTS: usize = 4;
const MENU_MANUAL: usize = 5;
const MENU_SYSCHECK: usize = 6;
// MENU[7] = EXIT: any other item goes back to the main screen

// LCD columns on the CALIBRATE screen ("R1P1 1.1V LOW HI")
const CAL_NAME_COL: u8 = 0;
const CAL_LOW_COL: u8 = 10;
const CAL_HI_COL: u8 = 14;

/// Selected column on the CALIBRATE screen
#[derive(Clone, Copy, PartialEq)]
enum CalCol {
    Name, // just picking a pod
    Low,  // click stores the current reading as LOW (dry)
    High, // click stores the current reading as HI (wet)
}

/// Selected field on the PROFILE screen
#[derive(Clone, Copy, PartialEq)]
enum PField {
    Plant,
    Water,
    Light,
}

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
    Light(usize), // pump off, only this plant's grow light on (checked by eye)
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Browse,
    Menu(usize),
    Profile { idx: usize, field: PField },
    Relearn { reads: u32, start_n: u32 }, // waiting for a fresh sensor reading + joystick samples
    RelearnResult,
    LearnPick { idx: usize },
    Learning,    // pot learning running (state in `learn`)
    LearnResult, // result in `learn`
    Calibrate { sel: usize, top: usize, col: CalCol }, // sel = selected line, top = first visible line
    TimeMenu { sel: usize },                            // 0 = clock, 1 = water time
    SetTime { what: TimeSet, hh: u8, mm: u8, field: TimeField },
    Faults { top: usize },
    Manual { sel: usize, top: usize },
    SysCheck { step: Sys, start_n: u32, start_pulses: u32 },
    SysResult,
}

impl Mode {
    /// Code for the signal trace (table in plant_logic/src/signals.rs)
    fn trace_code(&self) -> u8 {
        match self {
            Mode::Browse => 0,
            Mode::Menu(_) => 2,
            Mode::Relearn { .. } => 3,
            Mode::RelearnResult => 4,
            Mode::SysCheck { .. } => 5,
            Mode::SysResult => 6,
            Mode::Calibrate { .. } => 7,
            Mode::SetTime { .. } => 8,
            Mode::Profile { .. } => 9,
            Mode::TimeMenu { .. } => 10,
            Mode::Faults { .. } => 11,
            Mode::Manual { .. } => 12,
            Mode::LearnPick { .. } => 13,
            Mode::Learning => 14,
            Mode::LearnResult => 15,
        }
    }
}

/// Queue a trace line for the serial port (dropped and counted if the queue is full)
fn send_trace(line: &Line<LINE_MAX>) {
    // Never send a cut-off line: it would shift every column after it.
    // (Can't happen: unit tests check every line fits in LINE_MAX.)
    if line.ok() {
        trace_out::send(line.as_bytes());
    }
}

/// Snapshot of every input and output, for the MANUAL screen
struct ManualIo {
    joy: (u16, u16),
    button: bool,
    level_closed: bool,
    flow_pulses: u32,
    raw: [u16; SENSORS],
    pump: bool,
    valves: [bool; VALVES],
    lights: [bool; LIGHTS],
    led: bool,
}

fn manual_value(item: ManualItem, io: &ManualIo) -> ManualValue {
    match item {
        ManualItem::JoyX => ManualValue::Raw(io.joy.0 as u32),
        ManualItem::JoyY => ManualValue::Raw(io.joy.1 as u32),
        ManualItem::Button => ManualValue::Raw(io.button as u32),
        ManualItem::LevelSwitch => ManualValue::Closed(io.level_closed),
        ManualItem::FlowPulses => ManualValue::Raw(io.flow_pulses),
        ManualItem::Hygro(s) => ManualValue::Raw(io.raw[s] as u32),
        ManualItem::Pump => ManualValue::On(io.pump),
        ManualItem::Valve(v) => ManualValue::On(io.valves[v]),
        ManualItem::Light(l) => ManualValue::On(io.lights[l]),
        ManualItem::Led => ManualValue::On(io.led),
    }
}

/// Save every setting (one flash record). The byte layout is in plant_logic::packing.
fn save_all(store: &mut Store, settings: &Settings) -> bool {
    store.save(&settings.to_payload())
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

        // Signal trace: PA2 = USART2 TX, wired to the ST-LINK's USB serial port.
        // Record it on the PC with tools/trace_record.py (see trace_out.rs).
        let trace_tx = dp
            .USART2
            .tx(gpioa.pa2, hal::serial::Config::default().baudrate(trace_out::BAUD.bps()), &mut rcc)
            .unwrap();
        trace_out::init(trace_tx);

        // Joystick inputs
        let x_pin = gpioa.pa0.into_analog();
        let y_pin = gpioa.pa1.into_analog();
        let sw = gpioa.pa10.into_pull_up_input(); // button pulls to GND when pressed

        // Moisture sensor inputs (sensors are always powered)
        let m0_pin = gpioa.pa4.into_analog(); // R1 P1
        let m1_pin = gpiob.pb0.into_analog(); // R1 P2
        let m2_pin = gpioc.pc1.into_analog(); // R1 P3
        let m3_pin = gpioc.pc0.into_analog(); // R2 P1
        let m4_pin = gpiob.pb1.into_analog(); // R2 P2
        let m5_pin = gpioc.pc2.into_analog(); // R2 P3

        // ADC1, 12-bit by default (shared by joystick and hygrometers)
        let mut adc = Adc::new(dp.ADC1, true, AdcConfig::default(), &mut rcc);

        // Fault LED
        let mut led_hmi = gpiob.pb5.into_push_pull_output();

        // Water level switch: PB3 to GND, internal pull-up (closed = pin low)
        let level_sw = gpiob.pb3.into_pull_up_input();
        let mut low_water = level_is_low(level_sw.is_low()); // start from the real state, no debounce wait
        let mut level_ticks: u32 = 0; // how long the switch has disagreed with `low_water`

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

        // Grow lights: off at startup, switched by the 1000 ms task. Index = light number (see LIGHT_OF).
        let mut lights = [
            gpioa.pa6.into_push_pull_output_in_state(PinState::Low).erase(),  // 0: R1 P1
            gpioa.pa7.into_push_pull_output_in_state(PinState::Low).erase(),  // 1: R1 P2
            gpiob.pb12.into_push_pull_output_in_state(PinState::Low).erase(), // 2: R1 P3
            gpiob.pb13.into_push_pull_output_in_state(PinState::Low).erase(), // 3: R2 P1
            gpiob.pb14.into_push_pull_output_in_state(PinState::Low).erase(), // 4: R2 P2
            gpiob.pb15.into_push_pull_output_in_state(PinState::Low).erase(), // 5: R2 P3
        ];

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

        // Settings: per-plant water / light / pot size, water time, calibration,
        // joystick center. Loaded from flash if saved before (older formats are
        // converted), otherwise the defaults (50 %, 12 h light, water at 09:00).
        let (mut store, saved) = Store::open(dp.FLASH, MAX_PLANTS as u8);
        let mut settings = match saved {
            Some((payload, version)) => Settings::from_payload(&payload, version),
            None => Settings::DEFAULT,
        };
        let mut save_failed = false;

        // UI state
        let mut sel_idx: usize = 0; // plant slot shown on the main screen
        let mut mode = Mode::Browse;
        let mut last_dir = Dir::Center;
        let mut hold_ticks: u16 = 0; // how long the stick has been held in one direction
        let mut press_ticks: u32 = 0; // how long the button has been held
        let mut cooldown: u8 = 0;
        let mut redraw = true;
        let mut drawing = false; // redraw in progress: line 1 done, line 2 on the next tick
        let mut msg: Option<&'static str> = None; // e.g. "BUSY: WATERING", on line 2 of the menu / pick screens
        let long_press_ticks: u32 = (P_LONG_PRESS_MS / TICK_MS).max(1);

        // Joystick center learning (RELEARN)
        let mut joy_sum: (u32, u32) = (0, 0);
        let mut joy_samples: u32 = 0;
        let mut joy_learn_ok = false; // result of the last RELEARN, for the result screen

        // Sensor state
        let mut moisture: [u8; SENSORS] = [0; SENSORS]; // percent, per sensor
        let mut detected: [bool; SENSORS] = [false; SENSORS]; // plugged in, from the last reading
        let mut seen: [bool; SENSORS] = [false; SENSORS]; // detected at some point since power-on / RELEARN
        let mut hygro_ok: [bool; SENSORS] = [false; SENSORS]; // detected AND calibrated (usable)
        let mut raw_last: [u16; SENSORS] = [0; SENSORS]; // last raw ADC reading, for calibration
        let mut cal_flash: Option<(usize, u32)> = None; // (line, tick) of the last capture, shows "SET!"
        let mut moisture_reads: u32 = 0; // completed readings, so watering can wait for a fresh one
        let mut seconds: u32 = 0; // uptime, counted by the 1000 ms task
        let mut next_moisture_s: u32 = 0; // read immediately at startup
        let mut last_total_pulses: u32 = 0;
        let mut flow_lpm: f32 = 0.0;
        let mut read_due = true; // take a moisture reading on the next 100 ms tick (first one at startup)

        // Clock: time of day = (seconds + clock_offset) % 86400, once set from the menu
        let mut clock_set = false;
        let mut clock_offset: u32 = 0;
        let mut last_water_day: Option<u32> = None; // day number of the last daily run

        // Watering state
        let mut water = Water::Standby;
        let mut pid = Pid::new(); // PID state of the pod being watered
        let mut last_dose_ml: f32 = 0.0; // last PID output, for the trace
        let mut water_run_due = false;
        let mut next_water_s: u32 = 0; // fallback schedule while the clock isn't set

        // Pot learning (RELEARN > pick a plant); Some while running and on its result screen
        let mut learn: Option<Learn> = None;

        // Faults (catalog and severities in plant_logic/src/faults.rs)
        let mut faults = Faults::new();
        let mut last_overrun_s: Option<u32> = None;

        // MANUAL: when the pump was switched on by hand (for its timeout)
        let mut manual_pump_s: u32 = 0;

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

        // Signal trace (list of signals: plant_logic/src/signals.rs)
        let mut uptime_ms: u64 = 0; // trace time stamp, never wraps
        let mut next_header_ms: u64 = 0; // headers at power-on, then every HEADER_EVERY_MS
        let mut trace_line: Line<LINE_MAX> = Line::new();

        // -----------------------------------------------------------------------------------------------------
        // -----------------------------------------------------------------------------------------------------

        loop
        {
            // Wait for the next 10 ms tick (wrap-safe)
            while (DWT::cycle_count().wrapping_sub(next) as i32) < 0 {}
            next = next.wrapping_add(tick);
            n = n.wrapping_add(1);
            uptime_ms += TICK_MS as u64;

            let tod = (seconds % SECONDS_PER_DAY + clock_offset) % SECONDS_PER_DAY; // time of day, s
            let learning = learn.as_ref().map_or(false, |l| !l.done());
            // Something that owns the pump / valves / lights right now (no automatic watering)
            let auto_paused = learning || matches!(mode, Mode::SysCheck { .. } | Mode::Manual { .. });
            // Free to start something that uses the pump
            let pump_free = water == Water::Standby && !learning;

        // -----------------------------------------------------------------------------
        // 10ms tasks
        // -----------------------------------------------------------------------------

            // -----------------------
            // Water level switch (debounced, so ripples in the tank don't toggle it)
            // -----------------------
            let low_now = level_is_low(level_sw.is_low());
            if low_now != low_water {
                level_ticks += 1;
                if level_ticks * TICK_MS >= P_LEVEL_DEBOUNCE_MS {
                    low_water = low_now;
                    level_ticks = 0;
                    redraw = true;
                }
            } else {
                level_ticks = 0;
            }

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
                    Mode::Browse => Mode::Menu(0),
                    Mode::SysCheck { .. } | Mode::Manual { .. } => {
                        // Abort / leave: everything off (the 1 s task puts the lights back on schedule)
                        pump.set_low();
                        for v in valves.iter_mut() {
                            v.set_low();
                        }
                        for l in lights.iter_mut() {
                            l.set_low();
                        }
                        led_hmi.set_low();
                        Mode::Browse
                    }
                    Mode::Learning => {
                        if let Some(l) = learn.as_mut() {
                            l.abort();
                        }
                        pump.set_low();
                        for v in valves.iter_mut() {
                            v.set_low();
                        }
                        Mode::LearnResult
                    }
                    _ => Mode::Browse, // any menu screen: back to the main screen
                };
                if learn.map_or(false, |l| l.done()) && mode != Mode::LearnResult {
                    learn = None; // result screen left
                }
                msg = None;
                redraw = true;
            }

            if click {
                msg = None;
                mode = match mode {
                    // Main screen: edit this plant
                    Mode::Browse => Mode::Profile { idx: sel_idx, field: PField::Water },

                    Mode::Menu(MENU_PROFILE) => Mode::Profile { idx: sel_idx, field: PField::Plant },
                    Mode::Profile { idx, .. } => {
                        sel_idx = idx; // done: show this plant on the main screen
                        Mode::Browse
                    }

                    Mode::Menu(MENU_RELEARN) => {
                        // Read all sensors right now, and start averaging the joystick
                        read_due = true;
                        joy_sum = (0, 0);
                        joy_samples = 0;
                        Mode::Relearn { reads: moisture_reads, start_n: n }
                    }
                    Mode::Relearn { reads, start_n } => Mode::Relearn { reads, start_n }, // still learning
                    Mode::RelearnResult => Mode::LearnPick { idx: sel_idx },
                    Mode::LearnPick { idx } => {
                        if !pump_free {
                            msg = Some("BUSY: WATERING");
                        } else if sensor_valve(idx, &hygro_ok).is_none() {
                            msg = Some("NO POD THERE");
                        }
                        if msg.is_some() {
                            Mode::LearnPick { idx }
                        } else {
                            learn = Some(Learn::new(idx, moisture_reads));
                            read_due = true; // starting moisture
                            Mode::Learning
                        }
                    }
                    Mode::Learning => Mode::Learning, // hold to abort
                    Mode::LearnResult => {
                        let idx = learn.map_or(sel_idx, |l| l.idx);
                        learn = None;
                        Mode::LearnPick { idx }
                    }

                    // Live values: the sensors are read every 100 ms while this screen is open
                    Mode::Menu(MENU_CALIBRATE) => Mode::Calibrate { sel: 0, top: 0, col: CalCol::Name },
                    Mode::Calibrate { sel, top, col } => {
                        // Store the current reading as LOW (dry) or HI (wet) for this sensor
                        if let Some((_, _, s)) = cal_entry(sel) {
                            if detected[s] && col != CalCol::Name {
                                match col {
                                    CalCol::Low => settings.cal_dry[s] = raw_last[s],
                                    CalCol::High => settings.cal_wet[s] = raw_last[s],
                                    CalCol::Name => {}
                                }
                                hygro_ok = usable_sensors(&detected, &settings.cal_dry, &settings.cal_wet);
                                save_failed = !save_all(&mut store, &settings);
                                skip_overrun = true;
                                cal_flash = Some((sel, n));
                            }
                        }
                        Mode::Calibrate { sel, top, col }
                    }

                    Mode::Menu(MENU_TIME) => Mode::TimeMenu { sel: 0 },
                    Mode::TimeMenu { sel: 0 } => {
                        let (hh, mm) = if clock_set { hh_mm(tod) } else { (12, 0) };
                        Mode::SetTime { what: TimeSet::Clock, hh, mm, field: TimeField::Hour }
                    }
                    Mode::TimeMenu { .. } => Mode::SetTime {
                        what: TimeSet::Water,
                        hh: settings.water_time.0,
                        mm: settings.water_time.1,
                        field: TimeField::Hour,
                    },
                    Mode::SetTime { what: TimeSet::Clock, hh, mm, .. } => {
                        // Clock now reads hh:mm:00
                        let set = hh as u32 * 3600 + mm as u32 * 60;
                        clock_offset = (set + SECONDS_PER_DAY - seconds % SECONDS_PER_DAY) % SECONDS_PER_DAY;
                        clock_set = true;
                        last_water_day = None;
                        Mode::TimeMenu { sel: 0 }
                    }
                    Mode::SetTime { what: TimeSet::Water, hh, mm, .. } => {
                        settings.water_time = (hh, mm);
                        save_failed = !save_all(&mut store, &settings);
                        skip_overrun = true;
                        Mode::TimeMenu { sel: 1 }
                    }

                    Mode::Menu(MENU_FAULTS) => Mode::Faults { top: 0 },
                    Mode::Faults { top } => Mode::Faults { top },

                    Mode::Menu(MENU_MANUAL) => {
                        if pump_free {
                            // Start from a known state: everything off
                            pump.set_low();
                            for v in valves.iter_mut() {
                                v.set_low();
                            }
                            for l in lights.iter_mut() {
                                l.set_low();
                            }
                            led_hmi.set_low();
                            Mode::Manual { sel: 0, top: 0 }
                        } else {
                            msg = Some("BUSY: WATERING");
                            Mode::Menu(MENU_MANUAL)
                        }
                    }
                    Mode::Manual { sel, top } => {
                        // Click on an output: switch it
                        match manual_item(sel) {
                            Some(ManualItem::Pump) => {
                                pump.toggle();
                                manual_pump_s = seconds;
                            }
                            Some(ManualItem::Valve(v)) => valves[v].toggle(),
                            Some(ManualItem::Light(l)) => lights[l].toggle(),
                            Some(ManualItem::Led) => led_hmi.toggle(),
                            _ => {} // inputs: nothing to switch
                        }
                        Mode::Manual { sel, top }
                    }

                    Mode::Menu(MENU_SYSCHECK) => {
                        if pump_free {
                            sys_result = [b'-'; PLANT_SLOTS];
                            sys_leak = false;
                            led_hmi.set_high();
                            Mode::SysCheck {
                                step: Sys::Led,
                                start_n: n,
                                start_pulses: FLOW_PULSES.load(Ordering::Relaxed),
                            }
                        } else {
                            msg = Some("BUSY: WATERING");
                            Mode::Menu(MENU_SYSCHECK)
                        }
                    }
                    Mode::SysCheck { step, start_n, start_pulses } => {
                        Mode::SysCheck { step, start_n, start_pulses } // hold to abort
                    }
                    Mode::SysResult => Mode::Menu(MENU_SYSCHECK),

                    Mode::Menu(_) => Mode::Browse, // EXIT
                };
                redraw = true;
            }


            // -----------------------
            // Joystick stuff here:
            // -----------------------
            // converting the analog inputs to digital values
            let x_raw: u16 = adc.convert(&x_pin, SampleTime::Cycles_480);
            let y_raw: u16 = adc.convert(&y_pin, SampleTime::Cycles_480);

            // RELEARN: average the resting stick position, then finish once the
            // sensor reading is also in
            if let Mode::Relearn { reads, start_n } = mode {
                if n.wrapping_sub(start_n) >= JOY_SETTLE_TICKS {
                    joy_sum.0 += x_raw as u32;
                    joy_sum.1 += y_raw as u32;
                    joy_samples += 1;
                }
                if moisture_reads != reads && joy_samples >= JOY_MIN_SAMPLES {
                    let center = (
                        (joy_sum.0 / joy_samples) as u16,
                        (joy_sum.1 / joy_samples) as u16,
                    );
                    // Reject a center that's way off (stick was being held)
                    joy_learn_ok = joy_center_ok(center.0) && joy_center_ok(center.1);
                    if joy_learn_ok {
                        settings.joy_center = center;
                        save_failed = !save_all(&mut store, &settings);
                        skip_overrun = true;
                    }
                    // A pod that is gone now was removed on purpose: no NO SENSOR fault
                    seen = detected;
                    mode = Mode::RelearnResult;
                    redraw = true;
                }
            }

            // Shift by the learned center before applying the direction thresholds
            // FS for joystick here, to set to 2048 if OFF
            let center = if FS_JOY_RESET { settings.joy_center } else { (JOY_IDEAL_CENTER, JOY_IDEAL_CENTER) };
            let x = joy_correct(x_raw, center.0);
            let y = joy_correct(y_raw, center.1);
            let dir = read_dir(x, y);

            // One step each time the stick leaves the center. Holding up/down
            // auto-repeats (after 0.5 s, every 150 ms) where values are changed.
            if dir != Dir::Center && dir == last_dir {
                hold_ticks = hold_ticks.saturating_add(1);
            } else {
                hold_ticks = 0;
            }
            let edge = dir != last_dir && last_dir == Dir::Center;
            let repeat = matches!(
                mode,
                Mode::SetTime { .. } | Mode::Profile { field: PField::Light, .. } | Mode::Faults { .. } | Mode::Manual { .. }
            ) && matches!(dir, Dir::Up | Dir::Down)
                && hold_ticks >= 50
                && (hold_ticks - 50) % 15 == 0;
            last_dir = dir;

            if edge || repeat {
                msg = None;
                match mode {
                    Mode::Browse => match dir {
                        Dir::Up | Dir::Left => sel_idx = prev_plant(sel_idx),
                        Dir::Down | Dir::Right => sel_idx = next_plant(sel_idx),
                        Dir::Center => {}
                    },
                    Mode::Menu(item) => {
                        mode = match dir {
                            Dir::Up => Mode::Menu((item + MENU.len() - 1) % MENU.len()),
                            Dir::Down => Mode::Menu((item + 1) % MENU.len()),
                            Dir::Left => Mode::Browse,
                            _ => mode,
                        };
                    }
                    Mode::Profile { idx, field } => {
                        let before = *settings.plant(idx);
                        mode = match (dir, field) {
                            (Dir::Up, PField::Plant) => Mode::Profile { idx: prev_plant(idx), field },
                            (Dir::Down, PField::Plant) => Mode::Profile { idx: next_plant(idx), field },
                            (Dir::Up, PField::Water) => {
                                settings.plant_mut(idx).water = water_up(before.water);
                                mode
                            }
                            (Dir::Down, PField::Water) => {
                                settings.plant_mut(idx).water = water_down(before.water);
                                mode
                            }
                            (Dir::Up, PField::Light) => {
                                settings.plant_mut(idx).light_h = light_up(before.light_h);
                                mode
                            }
                            (Dir::Down, PField::Light) => {
                                settings.plant_mut(idx).light_h = light_down(before.light_h);
                                mode
                            }
                            (Dir::Right, PField::Plant) => Mode::Profile { idx, field: PField::Water },
                            (Dir::Right, _) => Mode::Profile { idx, field: PField::Light },
                            (Dir::Left, PField::Light) => Mode::Profile { idx, field: PField::Water },
                            (Dir::Left, PField::Water) => Mode::Profile { idx, field: PField::Plant },
                            (Dir::Left, PField::Plant) => Mode::Menu(MENU_PROFILE),
                            (Dir::Center, _) => mode,
                        };
                        // Save every change (pushing past the ends changes nothing, so no save)
                        if *settings.plant(idx) != before {
                            save_failed = !save_all(&mut store, &settings);
                            skip_overrun = true; // rare 1-2 s sector erase is expected, not a fault
                        }
                    }
                    Mode::RelearnResult if dir == Dir::Left => mode = Mode::Menu(MENU_RELEARN),
                    Mode::RelearnResult if dir == Dir::Right => mode = Mode::LearnPick { idx: sel_idx },
                    Mode::LearnPick { idx } => {
                        mode = match dir {
                            Dir::Up => Mode::LearnPick { idx: prev_plant(idx) },
                            Dir::Down => Mode::LearnPick { idx: next_plant(idx) },
                            Dir::Left => Mode::Menu(MENU_RELEARN),
                            _ => mode,
                        };
                    }
                    Mode::LearnResult if dir == Dir::Left => {
                        let idx = learn.map_or(sel_idx, |l| l.idx);
                        learn = None;
                        mode = Mode::LearnPick { idx };
                    }
                    Mode::SysResult if dir == Dir::Left => mode = Mode::Menu(MENU_SYSCHECK),
                    Mode::Calibrate { sel, top, col } => {
                        let count = cal_count().max(1);
                        mode = match (dir, col) {
                            (Dir::Up, _) => {
                                let sel = (sel + count - 1) % count;
                                Mode::Calibrate { sel, top: cal_scroll(sel, top), col }
                            }
                            (Dir::Down, _) => {
                                let sel = (sel + 1) % count;
                                Mode::Calibrate { sel, top: cal_scroll(sel, top), col }
                            }
                            (Dir::Right, CalCol::Name) => Mode::Calibrate { sel, top, col: CalCol::Low },
                            (Dir::Right, _) => Mode::Calibrate { sel, top, col: CalCol::High },
                            (Dir::Left, CalCol::High) => Mode::Calibrate { sel, top, col: CalCol::Low },
                            (Dir::Left, CalCol::Low) => Mode::Calibrate { sel, top, col: CalCol::Name },
                            (Dir::Left, CalCol::Name) => Mode::Menu(MENU_CALIBRATE),
                            (Dir::Center, _) => mode,
                        };
                    }
                    Mode::TimeMenu { sel } => {
                        mode = match dir {
                            Dir::Up | Dir::Down => Mode::TimeMenu { sel: 1 - sel },
                            Dir::Left => Mode::Menu(MENU_TIME),
                            _ => mode,
                        };
                    }
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
                    Mode::Faults { top } => {
                        // Scroll the 2-line window through the list
                        let last_top = faults.line_count().saturating_sub(2);
                        let top = top.min(last_top);
                        mode = match dir {
                            Dir::Up => Mode::Faults { top: top.saturating_sub(1) },
                            Dir::Down => Mode::Faults { top: (top + 1).min(last_top) },
                            Dir::Left => Mode::Menu(MENU_FAULTS),
                            _ => mode,
                        };
                    }
                    Mode::Manual { sel, top } => {
                        mode = match dir {
                            Dir::Up => {
                                let sel = (sel + MANUAL_COUNT - 1) % MANUAL_COUNT;
                                Mode::Manual { sel, top: cal_scroll(sel, top) }
                            }
                            Dir::Down => {
                                let sel = (sel + 1) % MANUAL_COUNT;
                                Mode::Manual { sel, top: cal_scroll(sel, top) }
                            }
                            Dir::Left => {
                                // Leaving: everything off, automatic control takes over again
                                pump.set_low();
                                for v in valves.iter_mut() {
                                    v.set_low();
                                }
                                for l in lights.iter_mut() {
                                    l.set_low();
                                }
                                led_hmi.set_low();
                                Mode::Menu(MENU_MANUAL)
                            }
                            _ => mode,
                        };
                    }
                    _ => {} // Relearn / Learning / SysCheck in progress: stick does nothing
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
                    None => match next_wired_light(0) {
                        Some(i) => Mode::SysCheck { step: Sys::Light(i), start_n: n, start_pulses: now_pulses },
                        None => Mode::SysResult,
                    },
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
                    Sys::Light(idx) if elapsed_ms >= P_SYSCHK_STEP_MS => {
                        mode = match next_wired_light(idx + 1) {
                            Some(i) => Mode::SysCheck { step: Sys::Light(i), start_n: n, start_pulses: now_pulses },
                            None => Mode::SysResult, // the 1 s task puts the lights back on schedule
                        };
                        redraw = true;
                    }
                    _ => {}
                }

                // Grow lights during the check: all off, except the one being tested
                let testing = match mode {
                    Mode::SysCheck { step: Sys::Light(i), .. } => wired_light(i),
                    _ => None,
                };
                for (l, pin) in lights.iter_mut().enumerate() {
                    if testing == Some(l) {
                        pin.set_high();
                    } else {
                        pin.set_low();
                    }
                }

                // Finished: the results become faults (until the next SYS CHECK)
                if mode == Mode::SysResult {
                    faults.set(FaultKind::Leak, None, sys_leak);
                    for idx in 0..PLANT_SLOTS {
                        if wired_valve(idx).is_some() {
                            faults.set(FaultKind::ValveBad, Some(idx), sys_result[idx] == b'X');
                        }
                    }
                }
            }


        // -----------------------------------------------------------------------------
        // 100ms tasks
        // tick 3: moisture read + display line 1
        // tick 4: display line 2 + cursor   (split so each tick stays inside 10 ms)
        // tick 5: watering control loop + pot learning
        // -----------------------------------------------------------------------------


            if n % 10 == 3
            {

                // Sensor: read when due, or every 100 ms on the live screens
                let live = matches!(mode, Mode::Calibrate { .. } | Mode::Manual { .. });
                if read_due || live {
                    read_due = false;
                    let raw: [u16; SENSORS] = [
                        read_avg!(adc, m0_pin),
                        read_avg!(adc, m1_pin),
                        read_avg!(adc, m2_pin),
                        read_avg!(adc, m3_pin),
                        read_avg!(adc, m4_pin),
                        read_avg!(adc, m5_pin),
                    ];

                    // Which sensors are plugged in, which are usable, and their moisture
                    raw_last = raw;
                    for i in 0..SENSORS {
                        detected[i] = hygro_detected(raw[i]);
                        seen[i] |= detected[i];
                    }
                    hygro_ok = usable_sensors(&detected, &settings.cal_dry, &settings.cal_wet);
                    for i in 0..SENSORS {
                        if let Some((dry, wet)) = cal_of(i, &settings.cal_dry, &settings.cal_wet) {
                            moisture[i] = moisture_pct(raw[i], dry, wet);
                        }
                    }
                    moisture_reads = moisture_reads.wrapping_add(1);
                    next_moisture_s = seconds + P_MOISTURE_PERIOD_S;
                    redraw = true;
                    // (RELEARN finishes in the 10 ms task, once the joystick samples are in too)
                }
                if matches!(mode, Mode::Learning) {
                    redraw = true; // live progress
                }

                // Display line 1
                drawing = redraw;
                redraw = false;
                if drawing {
                    match mode {
                        Mode::Browse => {
                            // Header; every other 2 s the worst active fault instead (not info-only ones)
                            let fault = faults.top().filter(|(k, _)| k.severity() < SEV_INFO);
                            match fault {
                                Some((k, pod)) if (seconds / 2) % 2 == 1 => show!(lcd, delay, 0, fault_text(k, pod).as_str()),
                                _ => show!(lcd, delay, 0, MAIN_HEADER),
                            }
                        }
                        Mode::Menu(_) => {
                            if clock_set {
                                let (h, m) = hh_mm(tod);
                                let _ = lcd.write_line(0, format_args!("MENU       {:02}:{:02}", h, m), &mut delay);
                            } else {
                                show!(lcd, delay, 0, "MENU       --:--");
                            }
                        }
                        Mode::Profile { idx, .. } => {
                            let (r, p) = row_plant(idx);
                            show!(lcd, delay, 0, profile_title(r, p).as_str());
                        }
                        Mode::Relearn { .. } => show!(lcd, delay, 0, "RELEARN..."),
                        Mode::RelearnResult => {
                            let found = (0..PLANT_SLOTS)
                                .filter(|&i| {
                                    let (r, p) = row_plant(i);
                                    plant_pod_installed(r, p, &hygro_ok)
                                })
                                .count();
                            let joy = if joy_learn_ok { "OK" } else { "ERR" };
                            let _ = lcd.write_line(0, format_args!("PODS:{} JOY:{}", found, joy), &mut delay);
                        }
                        Mode::LearnPick { .. } => show!(lcd, delay, 0, LEARN_PICK_TITLE),
                        Mode::Learning => {
                            if let Some(l) = learn.as_ref() {
                                show!(lcd, delay, 0, learn_title(l).as_str());
                            }
                        }
                        Mode::LearnResult => {
                            if let Some(Learn { idx, phase: Phase::Done(result), .. }) = learn {
                                show!(lcd, delay, 0, learn_result(idx, result).0.as_str());
                            }
                        }
                        Mode::Calibrate { top, .. } => {
                            let just_set = cal_flash.map_or(false, |(i, t)| i == top && n.wrapping_sub(t) < 100);
                            let line = cal_text(top, &detected, &raw_last, just_set);
                            show!(lcd, delay, 0, line.as_str());
                        }
                        Mode::TimeMenu { sel } => {
                            let now = if clock_set { Some(hh_mm(tod)) } else { None };
                            show!(lcd, delay, 0, time_line(sel == 0, "CLOCK", now).as_str());
                        }
                        Mode::SetTime { what: TimeSet::Clock, .. } => show!(lcd, delay, 0, "SET CLOCK"),
                        Mode::SetTime { what: TimeSet::Water, .. } => show!(lcd, delay, 0, "SET WATER TIME"),
                        Mode::Faults { top } => {
                            // (faults can clear while the list is open: keep the window inside it)
                            let top = top.min(faults.line_count().saturating_sub(2));
                            if let Some(line) = faults.line(top) {
                                show!(lcd, delay, 0, fault_line_text(line).as_str());
                            }
                        }
                        Mode::Manual { sel, top } => {
                            if let Some(item) = manual_item(top) {
                                let io = ManualIo {
                                joy: (x_raw, y_raw),
                                button: pressed,
                                level_closed: level_sw.is_low(),
                                flow_pulses: FLOW_PULSES.load(Ordering::Relaxed),
                                raw: raw_last,
                                pump: pump.is_set_high(),
                                valves: core::array::from_fn(|i| valves[i].is_set_high()),
                                lights: core::array::from_fn(|i| lights[i].is_set_high()),
                                led: led_hmi.is_set_high(),
                            };
                                let v = manual_value(item, &io);
                                show!(lcd, delay, 0, manual_text(item, sel == top, v).as_str());
                            }
                        }
                        Mode::SysCheck { step, .. } => match step {
                            Sys::Led => show!(lcd, delay, 0, "SYS CHECK: LED"),
                            Sys::PumpOnly => show!(lcd, delay, 0, "SYS CHECK: PUMP"),
                            Sys::Valve(idx) => {
                                let (r, p) = row_plant(idx);
                                let _ = lcd.write_line(0, format_args!("SYS CHK: R{} P{}", r + 1, p + 1), &mut delay);
                            }
                            Sys::Light(idx) => {
                                let (r, p) = row_plant(idx);
                                let _ = lcd.write_line(0, format_args!("SYS LIGHT: R{} P{}", r + 1, p + 1), &mut delay);
                            }
                        },
                        Mode::SysResult => {
                            let fails = sys_result.iter().filter(|&&c| c == b'X').count();
                            if sys_leak {
                                show!(lcd, delay, 0, "LEAK? PUMP ONLY");
                            } else if fails > 0 {
                                let _ = lcd.write_line(0, format_args!("SYS FAIL: {}", fails), &mut delay);
                            } else {
                                show!(lcd, delay, 0, "SYS CHECK OK");
                            }
                        }
                    }
                }
            }

            if n % 10 == 4 && drawing
            {
                // Display line 2 + cursor
                let mut cursor: Option<(u8, u8)> = None; // blinking cursor (column, line)
                match mode {
                    Mode::Browse => {
                        // The selected plant: moisture now, target, state
                        let (r, p) = row_plant(sel_idx);
                        let cfg = *settings.plant(sel_idx);
                        let installed = plant_pod_installed(r, p, &hygro_ok);
                        let act = match SENSOR_OF[r][p] {
                            Some(s) if installed => Some(moisture[s]),
                            _ => None,
                        };
                        let learning_here = learning && learn.map_or(false, |l| l.idx == sel_idx);
                        let state = pod_state(
                            sel_idx,
                            installed,
                            cfg.water,
                            act.unwrap_or(0),
                            &water,
                            learning_here,
                            faults.pod_has_fault(sel_idx),
                        );
                        show!(lcd, delay, 1, main_text(r, p, act, cfg.water, state).as_str());
                    }
                    Mode::Menu(item) => {
                        let text = msg.unwrap_or(MENU[item]);
                        let _ = lcd.write_line(1, format_args!("> {}", text), &mut delay);
                    }
                    Mode::Profile { idx, field } => {
                        show!(lcd, delay, 1, profile_values(settings.plant(idx)).as_str());
                        cursor = Some(match field {
                            PField::Plant => (PROFILE_PLANT_COL, 0),
                            PField::Water => (PROFILE_WATER_COL, 1),
                            PField::Light => (PROFILE_LIGHT_COL, 1),
                        });
                    }
                    Mode::Relearn { .. } => show!(lcd, delay, 1, "HANDS OFF STICK"),
                    Mode::RelearnResult => {
                        // 1 = pod installed, - = not, per plant, rows split by '|';
                        // click (or right) goes on to pot learning
                        let (buf, len) = plant_map(|i| {
                            let (r, p) = row_plant(i);
                            if plant_pod_installed(r, p, &hygro_ok) { b'1' } else { b'-' }
                        });
                        let map = core::str::from_utf8(&buf[..len]).unwrap_or("?");
                        let _ = lcd.write_line(1, format_args!("{}  POT>", map), &mut delay);
                    }
                    Mode::LearnPick { idx } => match msg {
                        Some(text) => show!(lcd, delay, 1, text),
                        None => {
                            let (r, p) = row_plant(idx);
                            show!(lcd, delay, 1, learn_pick_text(r, p, settings.plant(idx)).as_str());
                        }
                    },
                    Mode::Learning => {
                        if let Some(l) = learn.as_ref() {
                            show!(lcd, delay, 1, learn_status(l, FLOW_PULSES.load(Ordering::Relaxed)).as_str());
                        }
                    }
                    Mode::LearnResult => {
                        if let Some(Learn { idx, phase: Phase::Done(result), .. }) = learn {
                            show!(lcd, delay, 1, learn_result(idx, result).1.as_str());
                        }
                    }
                    Mode::Calibrate { sel, top, col } => {
                        let just_set = cal_flash.map_or(false, |(i, t)| i == top + 1 && n.wrapping_sub(t) < 100);
                        let line = cal_text(top + 1, &detected, &raw_last, just_set);
                        show!(lcd, delay, 1, line.as_str());
                        let x = match col {
                            CalCol::Name => CAL_NAME_COL,
                            CalCol::Low => CAL_LOW_COL,
                            CalCol::High => CAL_HI_COL,
                        };
                        cursor = Some((x, (sel - top) as u8));
                    }
                    Mode::TimeMenu { sel } => {
                        show!(lcd, delay, 1, time_line(sel == 1, "WATER", Some(settings.water_time)).as_str());
                    }
                    Mode::SetTime { hh, mm, field, .. } => {
                        let _ = lcd.write_line(1, format_args!("     {:02}:{:02}", hh, mm), &mut delay);
                        cursor = Some((
                            match field {
                                TimeField::Hour => HOUR_COL,
                                TimeField::Minute => MIN_COL,
                            },
                            1,
                        ));
                    }
                    Mode::Faults { top } => match faults.line(top.min(faults.line_count().saturating_sub(2)) + 1) {
                        Some(line) => show!(lcd, delay, 1, fault_line_text(line).as_str()),
                        None => show!(lcd, delay, 1, ""),
                    },
                    Mode::Manual { sel, top } => {
                        if let Some(item) = manual_item(top + 1) {
                            let io = ManualIo {
                                joy: (x_raw, y_raw),
                                button: pressed,
                                level_closed: level_sw.is_low(),
                                flow_pulses: FLOW_PULSES.load(Ordering::Relaxed),
                                raw: raw_last,
                                pump: pump.is_set_high(),
                                valves: core::array::from_fn(|i| valves[i].is_set_high()),
                                lights: core::array::from_fn(|i| lights[i].is_set_high()),
                                led: led_hmi.is_set_high(),
                            };
                            let v = manual_value(item, &io);
                            show!(lcd, delay, 1, manual_text(item, sel == top + 1, v).as_str());
                        }
                    }
                    Mode::SysCheck { .. } => show!(lcd, delay, 1, "HOLD BTN: ABORT"),
                    Mode::SysResult => {
                        // 1 = valve flow ok, X = failed, - = no valve
                        let (buf, len) = plant_map(|i| sys_result[i]);
                        let map = core::str::from_utf8(&buf[..len]).unwrap_or("?");
                        show!(lcd, delay, 1, map);
                    }
                }

                // Blinking cursor on the field being edited
                match cursor {
                    Some((col, line)) => {
                        let _ = lcd.set_cursor(col, line, &mut delay);
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
                water = match water {
                    // ---------- STANDBY: wait for the next watering run ----------
                    Water::Standby => {
                        if water_run_due && moisture_reads > 0 && !auto_paused {
                            water_run_due = false;
                            next_pod(0, &hygro_ok, &mut pid)
                        } else {
                            Water::Standby
                        }
                    }

                    // ---------- EVALUATE: PID decides the next dose ----------
                    Water::Evaluate { idx, cycle } => {
                        let cfg = *settings.plant(idx);
                        match sensor_valve(idx, &hygro_ok) {
                            Some((s, _)) => match decide(cfg.water, moisture[s], cycle, &mut pid, cfg.pot_ml_per_pct()) {
                                Decision::Dose(ml) => {
                                    last_dose_ml = ml;
                                    pump.set_high(); // PRE-CONTROL: pump on, valve still closed
                                    Water::Active {
                                        idx,
                                        cycle,
                                        sub: ActiveSub::PreControl,
                                        start_n: n,
                                        start_s: seconds,
                                        start_pulses: 0,
                                        goal: ml_to_pulses(ml),
                                    }
                                }
                                Decision::Done => {
                                    faults.set(FaultKind::NotWet, Some(idx), false);
                                    next_pod(idx + 1, &hygro_ok, &mut pid)
                                }
                                Decision::NotReached => {
                                    faults.set(FaultKind::NotWet, Some(idx), true);
                                    next_pod(idx + 1, &hygro_ok, &mut pid)
                                }
                            },
                            None => next_pod(idx + 1, &hygro_ok, &mut pid), // pod not installed: skip
                        }
                    }

                    // ---------- ACTIVE / PRE-CONTROL: build pressure ----------
                    Water::Active { idx, cycle, sub: ActiveSub::PreControl, start_n, goal, .. } => {
                        let (r, p) = row_plant(idx);
                        if !plant_pod_installed(r, p, &hygro_ok) {
                            Water::Slowdown { idx, cycle, start_n: n, end: RunEnd::Removed }
                        } else if n.wrapping_sub(start_n) * TICK_MS >= P_PRECONTROL_MS {
                            if let Some(v) = valve_of(idx) {
                                valves[v].set_high(); // FLOW ACTIVE: valve opens
                            }
                            Water::Active {
                                idx,
                                cycle,
                                sub: ActiveSub::FlowActive,
                                start_n: n,
                                start_s: seconds,
                                start_pulses: FLOW_PULSES.load(Ordering::Relaxed),
                                goal,
                            }
                        } else {
                            water
                        }
                    }

                    // ---------- ACTIVE / FLOW ACTIVE: deliver the dose ----------
                    Water::Active { idx, cycle, sub: ActiveSub::FlowActive, start_s, start_pulses, goal, .. } => {
                        let delivered = FLOW_PULSES.load(Ordering::Relaxed).wrapping_sub(start_pulses);
                        let run_s = seconds - start_s;
                        let no_flow = run_s >= P_FLOW_CHECK_S && flow_lpm < FLOW_SHOW_MIN_LPM;
                        let (r, p) = row_plant(idx);
                        let removed = !plant_pod_installed(r, p, &hygro_ok);

                        let end = if no_flow {
                            Some(RunEnd::NoFlow)
                        } else if removed {
                            Some(RunEnd::Removed)
                        } else if delivered >= goal || run_s >= P_PUMP_MAX_S {
                            Some(RunEnd::Done)
                        } else {
                            None
                        };
                        match end {
                            Some(end) => {
                                if let Some(v) = valve_of(idx) {
                                    valves[v].set_low(); // SLOWDOWN: valve closes first...
                                }
                                Water::Slowdown { idx, cycle, start_n: n, end }
                            }
                            None => water,
                        }
                    }

                    // ---------- SLOWDOWN: ...then the pump stops ----------
                    Water::Slowdown { idx, cycle, start_n, end } => {
                        if n.wrapping_sub(start_n) * TICK_MS >= P_SLOWDOWN_MS {
                            pump.set_low();
                            if let Some(v) = valve_of(idx) {
                                valves[v].set_low(); // make sure (e.g. ended during PRE-CONTROL)
                            }
                            match end {
                                RunEnd::Done => {
                                    faults.set(FaultKind::NoFlow, Some(idx), false); // water moved
                                    read_due = true; // first HOLD reading right away
                                    Water::Hold {
                                        idx,
                                        cycle: cycle + 1,
                                        start_s: seconds,
                                        reads: moisture_reads,
                                        last: None,
                                        steady: 0,
                                        next_sample_s: seconds + P_HOLD_SAMPLE_S,
                                    }
                                }
                                RunEnd::NoFlow => {
                                    faults.set(FaultKind::NoFlow, Some(idx), true);
                                    next_pod(idx + 1, &hygro_ok, &mut pid)
                                }
                                RunEnd::Removed => next_pod(idx + 1, &hygro_ok, &mut pid),
                            }
                        } else {
                            water
                        }
                    }

                    // ---------- HOLD: wait for moisture to settle ----------
                    Water::Hold { idx, cycle, start_s, reads, last, steady, next_sample_s } => {
                        let (r, p) = row_plant(idx);
                        let sensor = SENSOR_OF[r][p];
                        let (mut last, mut steady, mut next_sample_s) = (last, steady, next_sample_s);

                        // New reading in: count how many in a row barely changed
                        let fresh = moisture_reads != reads;
                        if fresh {
                            if let Some(s) = sensor {
                                let now = moisture[s];
                                steady = match last {
                                    Some(prev) if now.abs_diff(prev) <= P_HOLD_STEADY_PCT => steady + 1,
                                    _ => 0,
                                };
                                last = Some(now);
                            }
                        }

                        if steady >= P_HOLD_STEADY_COUNT || seconds - start_s >= P_HOLD_MAX_S || sensor.is_none() {
                            Water::Evaluate { idx, cycle } // settled (or gave up waiting): next PID step
                        } else {
                            if seconds >= next_sample_s {
                                read_due = true;
                                next_sample_s = seconds + P_HOLD_SAMPLE_S;
                            }
                            Water::Hold { idx, cycle, start_s, reads: moisture_reads, last, steady, next_sample_s }
                        }
                    }
                };

                // -------------------------------------------------
                // Pot learning (one step per 100 ms, see plant_logic/src/learn.rs)
                // -------------------------------------------------
                if let Some(l) = learn.as_mut() {
                    if !l.done() {
                        let (r, p) = row_plant(l.idx);
                        let pct = match SENSOR_OF[r][p] {
                            Some(s) if plant_pod_installed(r, p, &hygro_ok) => Some(moisture[s]),
                            _ => None,
                        };
                        let out = l.step(LearnIn {
                            now_ms: n.wrapping_mul(TICK_MS),
                            pulses: FLOW_PULSES.load(Ordering::Relaxed),
                            reads: moisture_reads,
                            pct,
                            flowing: flow_lpm >= FLOW_SHOW_MIN_LPM,
                        });
                        if out.pump {
                            pump.set_high();
                        } else {
                            pump.set_low();
                        }
                        if let Some(v) = valve_of(l.idx) {
                            if out.valve {
                                valves[v].set_high();
                            } else {
                                valves[v].set_low();
                            }
                        }
                        if out.want_read {
                            read_due = true;
                        }

                        // Finished: save a learned size, or raise the fault
                        if let Phase::Done(result) = l.phase {
                            match result {
                                Ok(ml_per_pct) => {
                                    settings.plant_mut(l.idx).ml_per_pct = ml_per_pct;
                                    save_failed = !save_all(&mut store, &settings);
                                    skip_overrun = true;
                                    faults.set(FaultKind::LearnFail, Some(l.idx), false);
                                }
                                Err(LearnError::Aborted) => {}
                                Err(e) => {
                                    faults.set(FaultKind::LearnFail, Some(l.idx), true);
                                    if e == LearnError::NoFlow {
                                        faults.set(FaultKind::NoFlow, Some(l.idx), true);
                                    }
                                }
                            }
                            if mode == Mode::Learning {
                                mode = Mode::LearnResult;
                            }
                            redraw = true;
                        }
                    }
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

                // Screens with a clock, live faults or moisture: keep them current
                if matches!(mode, Mode::Browse | Mode::Menu(_) | Mode::TimeMenu { .. } | Mode::Faults { .. }) {
                    redraw = true;
                }

                // Watering schedule
                if clock_set {
                    // Daily, in the minute of the WATER TIME
                    let t = seconds as u64 + clock_offset as u64;
                    let now_tod = (t % SECONDS_PER_DAY as u64) as u32;
                    let day = (t / SECONDS_PER_DAY as u64) as u32;
                    let start = settings.water_time.0 as u32 * 3600 + settings.water_time.1 as u32 * 60;
                    if now_tod >= start && now_tod < start + 60 && last_water_day != Some(day) {
                        last_water_day = Some(day);
                        water_run_due = true;
                    }
                } else if seconds >= next_water_s && moisture_reads > 0 {
                    // Clock not set (e.g. after a power loss): fall back to a fixed interval
                    next_water_s = seconds + P_WATER_INTERVAL_S;
                    water_run_due = true;
                }

                // Grow lights: each installed pod is lit for its light hours per day.
                // With the clock set the day starts at P_LIGHT_START_H; without it
                // (e.g. after a power loss) the light period starts at power-up.
                // (SYS CHECK and MANUAL switch the lights themselves while open)
                let (light_now, light_start) = if clock_set {
                    (tod, P_LIGHT_START_H as u32 * 3600)
                } else {
                    (seconds % SECONDS_PER_DAY, 0)
                };
                if !matches!(mode, Mode::SysCheck { .. } | Mode::Manual { .. }) {
                    for r in 0..ROWS {
                        for p in 0..PLANTS_PER_ROW[r] {
                            if let Some(l) = LIGHT_OF[r][p] {
                                let len = settings.plants[r][p].light_h as u32 * 3600;
                                let on = plant_pod_installed(r, p, &hygro_ok)
                                    && in_daily_window(light_now, light_start, len);
                                if on {
                                    lights[l].set_high();
                                } else {
                                    lights[l].set_low();
                                }
                            }
                        }
                    }
                }

                // MANUAL: a pump left on by hand turns itself off
                if matches!(mode, Mode::Manual { .. })
                    && pump.is_set_high()
                    && seconds - manual_pump_s >= P_MANUAL_PUMP_MAX_S
                {
                    pump.set_low();
                    redraw = true;
                }

                // Faults that follow a live condition (the rest are set where they happen)
                faults.set(FaultKind::LowWater, None, low_water);
                faults.set(FaultKind::SaveFail, None, save_failed);
                faults.set(FaultKind::NoClock, None, !clock_set);
                faults.set(
                    FaultKind::Overrun,
                    None,
                    last_overrun_s.map_or(false, |t| seconds - t < OVERRUN_FAULT_S),
                );
                for idx in 0..PLANT_SLOTS {
                    let (r, p) = row_plant(idx);
                    if let (true, Some(s)) = (plant_exists(idx), SENSOR_OF[r][p]) {
                        faults.set(FaultKind::NoSensor, Some(idx), seen[s] && !detected[s]);
                    }
                }

                // Sensor: schedule the periodic reading
                if seconds >= next_moisture_s {
                    read_due = true;
                }

                // (Case LED fault patterns are driven every 10 ms below, so faults
                // can flash faster than this 1 s task)
            }

            // -------------------------------------------------
            // FAULT logic: case LED, by the worst active fault
            //   severity 1-2  fast flash, 250 ms on / 250 ms off
            //   severity 3    slow blink, 1 s on / 1 s off
            //   4 / none      off
            // (SYS CHECK and MANUAL drive the LED themselves)
            // -------------------------------------------------
            if !matches!(mode, Mode::SysCheck { .. } | Mode::Manual { .. }) {
                let led = match faults.top().map(|(k, _)| k.severity()) {
                    Some(SEV_CRITICAL) | Some(SEV_HIGH) => (n / 25) % 2 == 0,
                    Some(SEV_WARNING) => (n / 100) % 2 == 0,
                    _ => false,
                };
                if led {
                    led_hmi.set_high();
                } else {
                    led_hmi.set_low();
                }
            }


            // -------------------------------------------------
            // Signal trace, at the end of the tick so it shows this tick's results.
            // The signals are declared in plant_logic/src/signals.rs; the compiler
            // makes sure every one is filled in here.
            // -------------------------------------------------
            if uptime_ms >= next_header_ms {
                next_header_ms = uptime_ms + HEADER_EVERY_MS;
                FastTrace::header(&mut trace_line);
                send_trace(&trace_line);
                ControlTrace::header(&mut trace_line);
                send_trace(&trace_line);
                SlowTrace::header(&mut trace_line);
                send_trace(&trace_line);
            }

            // 10 ms: inputs
            FastTrace {
                joy_x: x_raw,
                joy_y: y_raw,
                joy_dir: dir.code(),
                button: pressed,
                low_water_raw: low_now,
                flow_pulses: FLOW_PULSES.load(Ordering::Relaxed),
            }
            .frame(uptime_ms, &mut trace_line);
            send_trace(&trace_line);

            // 100 ms: control loop, outputs, sensors (same tick as the watering step)
            if n % 10 == 5 {
                let (goal_pulses, delivered_pulses) = match water {
                    Water::Active { sub: ActiveSub::FlowActive, start_pulses, goal, .. } => {
                        (goal, FLOW_PULSES.load(Ordering::Relaxed).wrapping_sub(start_pulses))
                    }
                    Water::Active { goal, .. } => (goal, 0),
                    _ => (0, 0),
                };
                let top = faults.top();
                ControlTrace {
                    mode: mode.trace_code(),
                    water_state: water.code(),
                    water_pod: idx_code(water.pod()),
                    water_cycle: water.cycle(),
                    dose_ml: last_dose_ml as u16,
                    pid_i_ml: pid.integral() as u16,
                    goal_pulses,
                    delivered_pulses,
                    pump: pump.is_set_high(),
                    valve: core::array::from_fn(|i| valves[i].is_set_high()),
                    led: led_hmi.is_set_high(),
                    low_water,
                    fault_top: top.map_or(0, |(k, _)| k.code()),
                    fault_pod: idx_code(top.and_then(|(_, pod)| pod)),
                    faults: faults.active_count() as u8,
                    learn: learn.map_or(0, |l| l.phase.code()),
                    learn_pod: idx_code(learn.map(|l| l.idx)),
                    save_failed,
                    overruns,
                    moisture_reads,
                    raw: raw_last,
                    moist: moisture,
                    detected,
                    usable: hygro_ok,
                    log_dropped: trace_out::dropped(),
                }
                .frame(uptime_ms, &mut trace_line);
                send_trace(&trace_line);
            }

            // 1 s: clock, flow, lights, user settings (same tick as the 1 s task)
            if n % 100 == 7 {
                SlowTrace {
                    uptime_s: seconds,
                    tod_s: (seconds % SECONDS_PER_DAY + clock_offset) % SECONDS_PER_DAY,
                    clock_set,
                    flow: (flow_lpm * 1000.0) as u32,
                    total_ml: pulses_to_ml(FLOW_PULSES.load(Ordering::Relaxed)),
                    light: core::array::from_fn(|i| lights[i].is_set_high()),
                    target: core::array::from_fn(|i| settings.plant(i).water),
                    light_h: core::array::from_fn(|i| settings.plant(i).light_h),
                    pot: core::array::from_fn(|i| settings.plant(i).pot_ml_per_pct()),
                    water_time_min: settings.water_time.0 as u16 * 60 + settings.water_time.1 as u16,
                    water_run_due,
                }
                .frame(uptime_ms, &mut trace_line);
                send_trace(&trace_line);
            }

            // Overrun check: this cycle ran past its 10 ms slot
            if skip_overrun {
                // After a flash save, restart the schedule from now instead of
                // racing through the ticks a sector erase may have skipped
                next = DWT::cycle_count().wrapping_add(tick);
                skip_overrun = false;
            } else if (DWT::cycle_count().wrapping_sub(next) as i32) >= 0 {
                overruns += 1;
                last_overrun_s = Some(seconds);
            }

        }

    }

    loop {}

}
