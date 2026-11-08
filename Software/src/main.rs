//! Demonstrate the use of a blocking `Delay` using the SYST (sysclock) timer.

/*
Hardware setup:
  STM32F446RE Nucleo board
  - Joystick X connected to PA0 / Arduino A0
  - Joystick Y connected to PA1 / Arduino A1
  - Button connected to PA10 (internal pull-up) / Arduino D2
  - LED Up connected to PB4 / Arduino D5
  - LED Left connected to PB5 / Arduino D4
  - LED Click connected to PB6 / Arduino D10

*/

#![deny(unsafe_code)]
#![allow(clippy::empty_loop)]
#![no_main]
#![no_std]

// Halt on panic
use panic_halt as _; // panic handler

use cortex_m_rt::entry;
use stm32f4xx_hal::{self as hal, adc::{ config::{AdcConfig, SampleTime}, Adc,}, rcc::Config,};
use crate::hal::{pac, prelude::*};

// 12-bit ADC: 0..=4095, joystick rests around ~2048.
// Readings below LOW or above HIGH count as "pushed".
const LOW: u16 = 1000;
#[allow(dead_code)]
const HIGH: u16 = 3000;

#[entry]
fn main() -> ! {
    if let (Some(dp), Some(cp)) = (
        pac::Peripherals::take(),
        cortex_m::peripheral::Peripherals::take(),
    ) {
        // Set up the system clock. We want to run at 48MHz for this one.
        let mut rcc = dp.RCC.freeze(Config::hsi().sysclk(48.MHz()));

        let gpioa = dp.GPIOA.split(&mut rcc);
        let gpiob = dp.GPIOB.split(&mut rcc);
 
        // Joystick inputs
        let x_pin = gpioa.pa0.into_analog();
        let y_pin = gpioa.pa1.into_analog();
        let sw = gpioa.pa10.into_pull_up_input(); // button pulls to GND when pressed
 
        // LEDs
        let mut led_up = gpiob.pb4.into_push_pull_output();
        let mut led_left = gpiob.pb5.into_push_pull_output();
        let mut led_click = gpiob.pb6.into_push_pull_output();

        // ADC1, 12-bit by default
        let mut adc = Adc::new(dp.ADC1, true, AdcConfig::default(), &mut rcc);

        // Create a delay abstraction based on SysTick
        let mut delay = cp.SYST.delay(&rcc.clocks);

        loop 
        {
            // converting the analog inputs to digital values
            let x: u16 = adc.convert(&x_pin, SampleTime::Cycles_480);
            let y: u16 = adc.convert(&y_pin, SampleTime::Cycles_480);

            // Up: Y pushed low. If it's backwards on your module, use `y > HIGH`.
            if y < LOW {
                led_up.set_high();
            } else {
                led_up.set_low();
            }
 
            // Left: X pushed low. If it's backwards, use `x > HIGH`.
            if x < LOW {
                led_left.set_high();
            } else {
                led_left.set_low();
            }
 
            // Click: active-low button
            if sw.is_low() {
                led_click.set_high();
            } else {
                led_click.set_low();
            }
 
            delay.delay_ms(10);

        }
        
    }

    loop {}
}