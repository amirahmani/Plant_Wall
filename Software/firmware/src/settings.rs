//! Plant settings saved in the STM32's own flash, so they survive power loss.
//!
//! Uses the last flash sector (sector 7: 128K at 0x0806_0000).
//! memory.x MUST keep program code out of it:  FLASH : ORIGIN = 0x08000000, LENGTH = 384K
//!
//! How it works: flash can only be rewritten after erasing a whole sector, which
//! takes 1-2 s on the F4 and wears the flash (~10,000 erases). So instead of
//! rewriting one spot, every save appends a new 64-byte record after the last one.
//! At startup the newest valid record is loaded. The sector is erased only when
//! all 2048 slots are used. Each record has a checksum, so a save cut off by a
//! power loss is ignored and the previous one is used instead.

use plant_logic::record::{decode, encode, PAYLOAD, RECORD_SIZE};
use stm32f4xx_hal::{flash::FlashExt, pac::FLASH};

// The record byte format (magic, version, checksum, ...) is in plant_logic/src/record.rs,
// where it is unit-tested. This file only deals with where records live in flash.
const SECTOR: u8 = 7;
const SECTOR_OFFSET: usize = 0x6_0000; // from the start of flash (0x0800_0000)
const SECTOR_SIZE: usize = 0x2_0000; // 128K
const SLOTS: usize = SECTOR_SIZE / RECORD_SIZE; // 2048 saves per erase

pub struct Store {
    flash: FLASH,
    next_slot: usize, // SLOTS means the sector is full
    tag: u8,
}

impl Store {
    /// Scan the sector. Returns the store and the newest saved (payload, version),
    /// or None if nothing valid has been saved yet.
    pub fn open(flash: FLASH, tag: u8) -> (Self, Option<([u8; PAYLOAD], u8)>) {
        let mut latest = None;
        let mut next_slot = SLOTS;
        {
            let mem = flash.read();
            let sector = &mem[SECTOR_OFFSET..SECTOR_OFFSET + SECTOR_SIZE];
            for slot in 0..SLOTS {
                let rec = &sector[slot * RECORD_SIZE..(slot + 1) * RECORD_SIZE];
                if rec.iter().all(|&b| b == 0xFF) {
                    next_slot = slot; // first never-written slot
                    break;
                }
                if let Some(found) = decode(rec, tag) {
                    latest = Some(found);
                }
                // anything else is a torn/old record: skipped
            }
        }
        (Store { flash, next_slot, tag }, latest)
    }

    /// Append one record. Returns false if the flash reported an error.
    /// Normally takes a few ms; once every 2048 saves the sector is erased first (1-2 s).
    pub fn save(&mut self, payload: &[u8; PAYLOAD]) -> bool {
        let rec = encode(payload, self.tag);
        let mut unlocked = self.flash.unlocked(); // re-locks when dropped
        if self.next_slot >= SLOTS {
            if unlocked.erase(SECTOR).is_err() {
                return false;
            }
            self.next_slot = 0;
        }
        let offset = SECTOR_OFFSET + self.next_slot * RECORD_SIZE;
        // Advance even on error, so a damaged slot is never written twice
        self.next_slot += 1;
        unlocked.program(offset, rec.iter()).is_ok()
    }
}
