

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
      fast flash (2 Hz) = LOW WATER, slow blink (0.5 Hz) = other fault
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
Grow lights, one per pod (GPIO out, HIGH = on). On/off for a set number of hours
per day from the pod's Lt setting, starting at P_LIGHT_START_H.
Lt is shown and set in hours, three steps from P_LIGHT_HOURS (8H / 12H / 16H).
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
    That way a broken or unplugged wire also shows the LOW WATER warning.
  - LOW WATER is a warning only: watering and SYS CHECK keep running.
    The case LED flashes fast and the LCD shows "LOW WATER REFILL" (when no
    watering status or fault is being shown). Clears by itself after a refill.
----------------------------------------------------------------------

Controls:
  Click (short press)  Browse: edit plant (installed pods only) / Edit: done / Menus: select
  Hold 1 s             Browse/Edit: open MENU / any menu screen: back to browse
                       during SYS CHECK: abort (pump off, all valves closed)
  Main screen:  "POD  NOW/SET  LT"   NOW = soil moisture from the last reading
                "R1P2  42/50% 12H"   SET = target, LT = light hours per day
  MENU (up/down to pick, click to select, left to go back):
    PROFILE     presets per plant type (herbs, greens, fern, ...) for the plant
                selected on the main screen: up/down picks one, click sets its
                water target + light and saves (list in plant_logic/src/plant.rs)
    RELEARN     re-read all sensors now and show which pods are found; with
                P_FS_JOY_RESET = 1 it also learns the joystick center (keep hands
                off the stick) and saves it
    SYS CHECK   LED 1 s, pump alone 0.5 s (leak check), then each valve 0.5 s with flow check
    CALIBRATE   live sensor voltages for all pods; up/down picks a pod, right moves to
                LOW (dry) / HI (wet), click stores the current reading (saved to flash)
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
// Everything hardware-independent (layout, calibration, PID, settings format, ...)
// lives in the plant_logic crate, where it is unit-tested on the PC.
use plant_logic::params::*;
use plant_logic::*;
use plant_logic::trace::{idx_code, Line, HEADER_EVERY_MS, LINE_MAX};
use crate::settings::Store;

// LCD constants
const LCD_ADDR: u8 = 0x27;

// (LCD columns of the Wtr and Lt values on the main screen: WATER_COL /
// LIGHT_COL in plant_logic/src/display.rs, next to the text they belong to)

// LCD columns of the hour and minute in the time editor ("     07:30")
const HOUR_COL: u8 = 5;
const MIN_COL: u8 = 8;

// Flow below this (L/min) counts as "no flow" (display and pump fault check)
// PARAMETER!! (move when .dcm integrated)
const FLOW_SHOW_MIN_LPM: f32 = 0.05;

// Daily water time used until one is saved from the WATER TIME menu
const DEFAULT_WATER_TIME: (u8, u8) = (7, 0);

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

// Joystick timing used by RELEARN (direction decoding is in plant_logic::joystick)
const JOY_SETTLE_TICKS: u32 = 100; // ignore the first 300 ms after the click (thumb leaving the stick)
const JOY_MIN_SAMPLES: u32 = 100; // at least 0.5 s of samples to average

// Function switch P_FS_JOY_RESET to enabl;e or diable the relearning funct. 
//   1 = ON, 0 = OFF, uses the 2048.
const FS_JOY_RESET: bool = P_FS_JOY_RESET != 0;
const _: () = assert!(P_FS_JOY_RESET <= 1, "P_FS_JOY_RESET must be 0 (off) or 1 (on)");

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

const MENU: [&str; 7] = ["PROFILE", "RELEARN", "SYS CHECK", "CALIBRATE", "CLOCK", "WATER TIME", "EXIT"];
const MENU_PROFILE: usize = 0;
const MENU_RELEARN: usize = 1;
const MENU_SYSCHECK: usize = 2;
const MENU_CALIBRATE: usize = 3;
const MENU_CLOCK: usize = 4;
const MENU_WATER_TIME: usize = 5;
// MENU[6] = EXIT: any other item goes back to browsing

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
    Relearn { reads: u32, start_n: u32 }, // waiting for a fresh sensor reading + joystick samples
    RelearnResult,
    SysCheck { step: Sys, start_n: u32, start_pulses: u32 },
    SysResult,
    Calibrate { sel: usize, top: usize, col: CalCol }, // sel = selected line, top = first visible line
    SetTime { what: TimeSet, hh: u8, mm: u8, field: TimeField },
    Profile { sel: usize }, // picking a PROFILES entry for the plant selected on the main screen
}

impl Mode {
    /// Code for the signal trace (table in plant_logic/src/signals.rs)
    fn trace_code(&self) -> u8 {
        match self {
            Mode::Browse => 0,
            Mode::Edit(_) => 1,
            Mode::Menu(_) => 2,
            Mode::Relearn { .. } => 3,
            Mode::RelearnResult => 4,
            Mode::SysCheck { .. } => 5,
            Mode::SysResult => 6,
            Mode::Calibrate { .. } => 7,
            Mode::SetTime { .. } => 8,
            Mode::Profile { .. } => 9,
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

/// Save plant settings, water time, menu calibration and joystick center together
/// (one flash record). The byte layout is in plant_logic::packing.
fn save_all(
    store: &mut Store,
    plants: &Plants,
    water_time: (u8, u8),
    cal_dry: &Cal,
    cal_wet: &Cal,
    joy_center: (u16, u16),
) -> bool {
    store.save(&pack(plants), &pack_extra(water_time, cal_dry, cal_wet, joy_center))
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

        // Plant settings: [row][plant] = (water, light). Sized for MAX_PLANTS per row,
        // only the first PLANTS_PER_ROW[row] entries are used.
        // Loaded from flash if saved before, otherwise every plant starts at 50 % / middle light step.
        let mut plants: Plants = [[(WATER_DEFAULT, LightLevel::Med); MAX_PLANTS]; ROWS];
        let mut water_time: (u8, u8) = DEFAULT_WATER_TIME; // daily watering time (HH, MM)
        // Calibration set from the CALIBRATE menu (CAL_UNSET = use params.dcm)
        let mut cal_dry: Cal = [CAL_UNSET; SENSORS];
        let mut cal_wet: Cal = [CAL_UNSET; SENSORS];
        // Joystick center learned by RELEARN (X, Y)
        let mut joy_center: (u16, u16) = (JOY_IDEAL_CENTER, JOY_IDEAL_CENTER);
        let (mut store, saved) = Store::open(dp.FLASH, MAX_PLANTS as u8);
        if let Some((cells, extra, version)) = saved {
            if version >= 4 {
                unpack(&cells, &mut plants);
            } else {
                unpack_v3(&cells, &mut plants); // older record: convert LOW/MED/HI to %
            }
            if extra[0] < 24 && extra[1] < 60 {
                water_time = (extra[0], extra[1]); // 0xFF = never set: keep default
            }
            (cal_dry, cal_wet) = unpack_cal(&extra);
            joy_center = unpack_joy(&extra);
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

        // Joystick center learning (RELEARN)
        let mut joy_sum: (u32, u32) = (0, 0);
        let mut joy_samples: u32 = 0;
        let mut joy_learn_ok = false; // result of the last RELEARN, for the result screen

        // Sensor state
        let mut moisture: [u8; SENSORS] = [0; SENSORS]; // percent, per sensor
        let mut detected: [bool; SENSORS] = [false; SENSORS]; // plugged in, from the last reading
        let mut hygro_ok: [bool; SENSORS] = [false; SENSORS]; // detected AND calibrated (usable)
        let mut raw_last: [u16; SENSORS] = [0; SENSORS]; // last raw ADC reading, for calibration
        let mut cal_flash: Option<(usize, u32)> = None; // (line, tick) of the last capture, shows "SET!"
        let mut moisture_reads: u32 = 0; // completed readings, so watering can wait for a fresh one
        let mut seconds: u32 = 0; // uptime, counted by the 1000 ms task
        let mut next_moisture_s: u32 = 0; // read immediately at startup
        let mut last_total_pulses: u32 = 0;
        let mut flow_lpm: f32 = 0.0;
        let mut total_liters: f32 = 0.0;
        let mut showing_flow = false;
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

        // -----------------------------------------------------------------------------
        // 10ms tasks
        // -----------------------------------------------------------------------------

            // -----------------------
            // Button: click on release (< 1 s), long press fires at 1 s while held
            // -----------------------
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

                    // Profiles apply to the plant selected on the main screen
                    Mode::Menu(MENU_PROFILE) => {
                        if plant_pod_installed(row, plant, &hygro_ok) {
                            let (water_set, light_set) = plants[row][plant];
                            Mode::Profile { sel: profile_of(water_set, light_set).unwrap_or(0) }
                        } else {
                            menu_msg = Some("NO POD SELECTED");
                            Mode::Menu(MENU_PROFILE)
                        }
                    }
                    Mode::Profile { sel } => {
                        let p = PROFILES[sel];
                        if plants[row][plant] != (p.water, p.light) {
                            plants[row][plant] = (p.water, p.light);
                            save_failed = !save_all(&mut store, &plants, water_time, &cal_dry, &cal_wet, joy_center);
                            skip_overrun = true;
                        }
                        Mode::Browse // shows the new settings
                    }

                    Mode::Menu(MENU_RELEARN) => {
                        // Read all sensors right now, and start averaging the joystick
                        read_due = true;
                        joy_sum = (0, 0);
                        joy_samples = 0;
                        Mode::Relearn { reads: moisture_reads, start_n: n }
                    }
                    Mode::Menu(MENU_SYSCHECK) => {
                        if water == Water::Standby {
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
                    // Live values: the sensors are read every 100 ms while this screen is open
                    Mode::Menu(MENU_CALIBRATE) => Mode::Calibrate { sel: 0, top: 0, col: CalCol::Name },
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

                    Mode::Relearn { reads, start_n } => Mode::Relearn { reads, start_n }, // still learning
                    Mode::RelearnResult => Mode::Menu(MENU_RELEARN),
                    Mode::SysCheck { step, start_n, start_pulses } => {
                        Mode::SysCheck { step, start_n, start_pulses } // hold to abort
                    }
                    Mode::SysResult => Mode::Menu(MENU_SYSCHECK),

                    Mode::Calibrate { sel, top, col } => {
                        // Store the current reading as LOW (dry) or HI (wet) for this sensor
                        if let Some((_, _, s)) = cal_entry(sel) {
                            if detected[s] && col != CalCol::Name {
                                match col {
                                    CalCol::Low => cal_dry[s] = raw_last[s],
                                    CalCol::High => cal_wet[s] = raw_last[s],
                                    CalCol::Name => {}
                                }
                                hygro_ok = usable_sensors(&detected, &cal_dry, &cal_wet);
                                save_failed = !save_all(&mut store, &plants, water_time, &cal_dry, &cal_wet, joy_center);
                                skip_overrun = true;
                                cal_flash = Some((sel, n));
                            }
                        }
                        Mode::Calibrate { sel, top, col }
                    }

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
                        save_failed = !save_all(&mut store, &plants, water_time, &cal_dry, &cal_wet, joy_center);
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
                        joy_center = center;
                        save_failed = !save_all(&mut store, &plants, water_time, &cal_dry, &cal_wet, joy_center);
                        skip_overrun = true;
                    }
                    mode = Mode::RelearnResult;
                    redraw = true;
                }
            }

            // Shift by the learned center before applying the direction thresholds
            // FS for joystick here, to set to 2048 if OFF
            let center = if FS_JOY_RESET { joy_center } else { (JOY_IDEAL_CENTER, JOY_IDEAL_CENTER) };
            let x = joy_correct(x_raw, center.0);
            let y = joy_correct(y_raw, center.1);
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
                            (Dir::Up, Field::Water) => entry.0 = water_up(entry.0),
                            (Dir::Down, Field::Water) => entry.0 = water_down(entry.0),
                            (Dir::Up, Field::Light) => entry.1 = entry.1.up(),
                            (Dir::Down, Field::Light) => entry.1 = entry.1.down(),
                            (Dir::Center, _) => {}
                        }
                        // Save every change (pushing past HI/LOW changes nothing, so no save)
                        if plants[row][plant] != before {
                            save_failed = !save_all(&mut store, &plants, water_time, &cal_dry, &cal_wet, joy_center);
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
                    Mode::Profile { sel } => {
                        mode = match dir {
                            Dir::Up => Mode::Profile { sel: (sel + PROFILES.len() - 1) % PROFILES.len() },
                            Dir::Down => Mode::Profile { sel: (sel + 1) % PROFILES.len() },
                            Dir::Left => Mode::Menu(MENU_PROFILE),
                            _ => mode,
                        };
                    }
                    Mode::RelearnResult if dir == Dir::Left => mode = Mode::Menu(MENU_RELEARN),
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

                // Sensor: read when due, or every 100 ms while calibrating
                {
                    if read_due || matches!(mode, Mode::Calibrate { .. }) {
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
                        }
                        hygro_ok = usable_sensors(&detected, &cal_dry, &cal_wet);
                        for i in 0..SENSORS {
                            if let Some((dry, wet)) = cal_of(i, &cal_dry, &cal_wet) {
                                moisture[i] = moisture_pct(raw[i], dry, wet);
                            }
                        }
                        moisture_reads = moisture_reads.wrapping_add(1);
                        next_moisture_s = seconds + P_MOISTURE_PERIOD_S;
                        redraw = true;

                        // Pod unplugged while editing it: back to browsing
                        if let Mode::Edit(_) = mode {
                            if !plant_pod_installed(row, plant, &hygro_ok) {
                                mode = Mode::Browse;
                            }
                        }
                        // (RELEARN finishes in the 10 ms task, once the joystick samples are in too)
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
                            let joy = if joy_learn_ok { "OK" } else { "ERR" };
                            let _ = lcd.write_line(0, format_args!("PODS:{} JOY:{}", found, joy), &mut delay);
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
                        Mode::Calibrate { top, .. } => {
                            let just_set = cal_flash.map_or(false, |(i, t)| i == top && n.wrapping_sub(t) < 100);
                            let line = cal_text(top, &detected, &raw_last, just_set);
                            let _ = lcd.write_line(0, format_args!("{}", line.as_str()), &mut delay);
                        }
                        Mode::Profile { .. } => {
                            let _ = lcd.write_line(0, format_args!("{}", profile_title(row, plant).as_str()), &mut delay);
                        }
                        Mode::SetTime { what: TimeSet::Clock, .. } => {
                            let _ = lcd.write_line(0, format_args!("SET CLOCK"), &mut delay);
                        }
                        Mode::SetTime { what: TimeSet::Water, .. } => {
                            let _ = lcd.write_line(0, format_args!("SET WATER TIME"), &mut delay);
                        }

                        // Browse / Edit, priority: save error > watering fault > watering status
                        //                          > low water warning > flow > header + this plant's moisture
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
                            } else if water != Water::Standby {
                                // Watering status, e.g. "R1P2 C1 45/150mL" or "R1P2 HOLD 42>50%"
                                match water {
                                    Water::Evaluate { idx, .. } => {
                                        let (r, p) = row_plant(idx);
                                        let _ = lcd.write_line(0, format_args!("R{}P{} CHECK", r + 1, p + 1), &mut delay);
                                    }
                                    Water::Active { idx, cycle, sub: ActiveSub::PreControl, .. } => {
                                        let (r, p) = row_plant(idx);
                                        let _ = lcd.write_line(0, format_args!("R{}P{} C{} PRESSURE", r + 1, p + 1, cycle + 1), &mut delay);
                                    }
                                    Water::Active { idx, cycle, sub: ActiveSub::FlowActive, start_pulses, goal, .. } => {
                                        let (r, p) = row_plant(idx);
                                        let delivered = FLOW_PULSES.load(Ordering::Relaxed).wrapping_sub(start_pulses);
                                        let _ = lcd.write_line(
                                            0,
                                            format_args!(
                                                "R{}P{} C{} {}/{}mL",
                                                r + 1,
                                                p + 1,
                                                cycle + 1,
                                                pulses_to_ml(delivered),
                                                pulses_to_ml(goal)
                                            ),
                                            &mut delay,
                                        );
                                    }
                                    Water::Slowdown { idx, cycle, .. } => {
                                        let (r, p) = row_plant(idx);
                                        let _ = lcd.write_line(0, format_args!("R{}P{} C{} SLOWDOWN", r + 1, p + 1, cycle + 1), &mut delay);
                                    }
                                    Water::Hold { idx, .. } => {
                                        let (r, p) = row_plant(idx);
                                        let now_pct = SENSOR_OF[r][p].map_or(0, |s| moisture[s]);
                                        let _ = lcd.write_line(
                                            0,
                                            format_args!("R{}P{} HOLD {}>{}%", r + 1, p + 1, now_pct, plants[r][p].0),
                                            &mut delay,
                                        );
                                    }
                                    Water::Standby => {}
                                }
                            } else if low_water {
                                // Warning only: shown when nothing more important is on screen
                                let _ = lcd.write_line(0, format_args!("LOW WATER REFILL"), &mut delay);
                            } else if showing_flow {
                                let _ = lcd.write_line(
                                    0,
                                    format_args!("{:>5.2}L/m {:>5.1}L", flow_lpm, total_liters),
                                    &mut delay,
                                );
                            } else {
                                // Column header for line 2 (the moisture itself is on line 2)
                                let header = if mode == Mode::Browse { BROWSE_HEADER } else { EDIT_HEADER };
                                let _ = lcd.write_line(0, format_args!("{}", header), &mut delay);
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
                    Mode::Menu(item) => {
                        let text = menu_msg.unwrap_or(MENU[item]);
                        let _ = lcd.write_line(1, format_args!("> {}", text), &mut delay);
                    }
                    Mode::Relearn { .. } => {
                        let _ = lcd.write_line(1, format_args!("HANDS OFF STICK"), &mut delay);
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
                    Mode::Calibrate { sel, top, col } => {
                        let just_set = cal_flash.map_or(false, |(i, t)| i == top + 1 && n.wrapping_sub(t) < 100);
                        let line = cal_text(top + 1, &detected, &raw_last, just_set);
                        let _ = lcd.write_line(1, format_args!("{}", line.as_str()), &mut delay);
                        let x = match col {
                            CalCol::Name => CAL_NAME_COL,
                            CalCol::Low => CAL_LOW_COL,
                            CalCol::High => CAL_HI_COL,
                        };
                        cursor = Some((x, (sel - top) as u8));
                    }
                    Mode::Profile { sel } => {
                        let _ = lcd.write_line(1, format_args!("{}", profile_text(&PROFILES[sel]).as_str()), &mut delay);
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
                    Mode::Browse | Mode::Edit(_) => {
                        // Selected plant: moisture now / target, light hours, e.g. "R1P2  42/50% 12H"
                        // ("--" for a pod that isn't installed, OFF = never watered automatically)
                        let (water_set, light_set) = plants[row][plant];
                        let now_pct = match SENSOR_OF[row][plant] {
                            Some(s) if plant_pod_installed(row, plant, &hygro_ok) => Some(moisture[s]),
                            _ => None,
                        };
                        let line = browse_text(row, plant, now_pct, water_set, light_set);
                        let _ = lcd.write_line(1, format_args!("{}", line.as_str()), &mut delay);
                        if let Mode::Edit(field) = mode {
                            cursor = Some((
                                match field {
                                    Field::Water => WATER_COL,
                                    Field::Light => LIGHT_COL,
                                },
                                1,
                            ));
                        }
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
                let sys_busy = matches!(mode, Mode::SysCheck { .. });
                water = match water {
                    // ---------- STANDBY: wait for the next watering run ----------
                    Water::Standby => {
                        if water_run_due && moisture_reads > 0 && !sys_busy {
                            water_run_due = false;
                            water_fault = None; // a new run clears the last run's fault
                            next_pod(0, &hygro_ok, &mut pid)
                        } else {
                            Water::Standby
                        }
                    }

                    // ---------- EVALUATE: PID decides the next dose ----------
                    Water::Evaluate { idx, cycle } => {
                        let (r, p) = row_plant(idx);
                        match sensor_valve(idx, &hygro_ok) {
                            Some((s, _)) => match decide(plants[r][p].0, moisture[s], cycle, &mut pid) {
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
                                Decision::Done => next_pod(idx + 1, &hygro_ok, &mut pid),
                                Decision::NotReached => {
                                    water_fault = Some(Fault::NotReached(idx));
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
                                    water_fault = Some(Fault::NoFlow(idx));
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

                if let Water::Active { .. } = water {
                    redraw = true; // live mL count while delivering
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
                if flowing || flowing != showing_flow || water != Water::Standby {
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

                // Grow lights: each installed pod is lit for its Lt hours per day.
                // With the clock set the day starts at P_LIGHT_START_H; without it
                // (e.g. after a power loss) the light period starts at power-up.
                let (light_now, light_start) = if clock_set {
                    (tod, P_LIGHT_START_H as u32 * 3600)
                } else {
                    (seconds % SECONDS_PER_DAY, 0)
                };
                for r in 0..ROWS {
                    for p in 0..PLANTS_PER_ROW[r] {
                        if let Some(l) = LIGHT_OF[r][p] {
                            let len = light_hours(plants[r][p].1) * 3600;
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

                // Sensor: schedule the periodic reading
                if seconds >= next_moisture_s {
                    read_due = true;
                }

                // (Case LED fault patterns are driven every 10 ms below, so LOW WATER
                // can flash faster than this 1 s task)
            }

            // -------------------------------------------------
            // FAULT logic: case LED (sys check drives the LED itself)
            //   LOW WATER    fast flash, 250 ms on / 250 ms off
            //   other fault  slow blink, 1 s on / 1 s off
            // -------------------------------------------------
            if !matches!(mode, Mode::SysCheck { .. }) {
                let fault = overruns > 0 || water_fault.is_some() || save_failed;
                let led = if low_water {
                    (n / 25) % 2 == 0
                } else if fault {
                    (n / 100) % 2 == 0
                } else {
                    false
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
                    fault: fault_code(water_fault),
                    fault_pod: idx_code(water_fault.map(|f| f.pod())),
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
                    target: core::array::from_fn(|i| {
                        let (r, p) = row_plant(i);
                        plants[r][p].0
                    }),
                    light_h: core::array::from_fn(|i| {
                        let (r, p) = row_plant(i);
                        light_hours(plants[r][p].1) as u8
                    }),
                    water_time_min: water_time.0 as u16 * 60 + water_time.1 as u16,
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
            }

        }

    }

    loop {}

}
