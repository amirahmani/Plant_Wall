//! Sends trace lines out of USART2 (PA2) -> ST-LINK -> USB serial on the PC
//! (/dev/ttyACM0 on Linux), without slowing the 10 ms loop down.
//!
//! The main loop hands over whole lines with `send()`: they are copied into a
//! queue in a few microseconds. The USART2 interrupt then sends the queue one byte at a
//! time in the background. If the queue is full (the PC can't keep up, which
//! shouldn't happen at 230400 baud), the line is dropped and counted instead of
//! blocking the control loop. Signal list and line format: plant_logic/src/signals.rs.

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m::interrupt::Mutex;
use plant_logic::trace::ByteQueue;
use stm32f4xx_hal::{
    pac::{self, interrupt, USART2},
    prelude::*,
    serial::Tx,
};

pub const BAUD: u32 = 230_400;

/// About 1.3 s of typical trace output, or all three headers twice over
const QUEUE_BYTES: usize = 4096;

static QUEUE: Mutex<RefCell<ByteQueue<QUEUE_BYTES>>> = Mutex::new(RefCell::new(ByteQueue::new()));
static TX: Mutex<RefCell<Option<Tx<USART2>>>> = Mutex::new(RefCell::new(None));
static DROPPED: AtomicU32 = AtomicU32::new(0);

/// Hand the configured transmitter to the interrupt and enable it.
pub fn init(tx: Tx<USART2>) {
    cortex_m::interrupt::free(|cs| TX.borrow(cs).replace(Some(tx)));
    enable_interrupt();
}

// Same reasoning as enable_flow_interrupt() in main.rs: the handler only
// touches data behind a Mutex, so unmasking it is safe.
#[allow(unsafe_code)]
fn enable_interrupt() {
    unsafe { cortex_m::peripheral::NVIC::unmask(pac::Interrupt::USART2) };
}

/// Queue one complete line. Returns false (and counts it) if it didn't fit.
pub fn send(line: &[u8]) -> bool {
    let queued = cortex_m::interrupt::free(|cs| {
        let ok = QUEUE.borrow(cs).borrow_mut().push_all(line);
        if ok {
            if let Some(tx) = TX.borrow(cs).borrow_mut().as_mut() {
                tx.listen(); // "transmit register empty" interrupt starts the sending
            }
        }
        ok
    });
    if !queued {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
    queued
}

/// Lines lost so far because the queue was full
pub fn dropped() -> u32 {
    DROPPED.load(Ordering::Relaxed)
}

// Fires whenever the USART can take another byte (while listening)
#[interrupt]
fn USART2() {
    cortex_m::interrupt::free(|cs| {
        let mut tx = TX.borrow(cs).borrow_mut();
        let Some(tx) = tx.as_mut() else { return };
        let mut queue = QUEUE.borrow(cs).borrow_mut();
        while tx.is_tx_empty() {
            match queue.pop() {
                Some(b) => {
                    let _ = tx.write(b);
                }
                None => {
                    tx.unlisten(); // nothing left: stop the interrupt until the next send()
                    break;
                }
            }
        }
    });
}
