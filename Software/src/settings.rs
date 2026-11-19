//! Plant settings saved in the STM32's own flash, so they survive power loss.
//!
//! Uses the last flash sector (sector 7: 128K at 0x0806_0000).
//! memory.x MUST keep program code out of it:  FLASH : ORIGIN = 0x08000000, LENGTH = 384K
//!
//! How it works: flash can only be rewritten after erasing a whole sector, which
//! takes 1-2 s on the F4 and wears the flash (~10,000 erases). So instead of
//! rewriting one spot, every save appends a new 32-byte record after the last one.
//! At startup the newest valid record is loaded. The sector is erased only when
//! all 4096 slots are used. Each record has a checksum, so a save cut off by a
//! power loss is ignored and the previous one is used instead.

use stm32f4xx_hal::{flash::FlashExt, pac::FLASH};

const SECTOR: u8 = 7;
const SECTOR_OFFSET: usize = 0x6_0000; // from the start of flash (0x0800_0000)
const SECTOR_SIZE: usize = 0x2_0000; // 128K
const RECORD_SIZE: usize = 32;
const SLOTS: usize = SECTOR_SIZE / RECORD_SIZE; // 4096 saves per erase

const MAGIC: [u8; 4] = *b"PLNT";
const VERSION: u8 = 1;

/// Bytes of plant data per record (one byte per plant).
pub const CELLS: usize = 16;
/// Extra settings bytes per record (currently: water time hour, minute).
pub const EXTRA: usize = 2;

// Record layout:
//   0..4   magic "PLNT"
//   4      format version
//   5      layout tag (MAX_PLANTS), so a different grid shape isn't misread
//   6..8   extra settings (water time HH, MM); 0xFF in older records = not set
//   8..24  plant cells
//   24..28 checksum of bytes 0..24
//   28..32 unused
const EXTRA_AT: usize = 6;
const CELLS_AT: usize = 8;
const CHECKSUM_AT: usize = CELLS_AT + CELLS;

pub struct Store {
    flash: FLASH,
    next_slot: usize, // SLOTS means the sector is full
    tag: u8,
}

impl Store {
    /// Scan the sector. Returns the store and the newest saved (cells, extra),
    /// or None if nothing valid has been saved yet.
    pub fn open(flash: FLASH, tag: u8) -> (Self, Option<([u8; CELLS], [u8; EXTRA])>) {
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
                if let Some(cells) = decode(rec, tag) {
                    latest = Some(cells);
                }
                // anything else is a torn/old record: skipped
            }
        }
        (Store { flash, next_slot, tag }, latest)
    }

    /// Append one record. Returns false if the flash reported an error.
    /// Normally takes a few ms; once every 4096 saves the sector is erased first (1-2 s).
    pub fn save(&mut self, cells: &[u8; CELLS], extra: &[u8; EXTRA]) -> bool {
        let rec = encode(cells, extra, self.tag);
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

fn encode(cells: &[u8; CELLS], extra: &[u8; EXTRA], tag: u8) -> [u8; RECORD_SIZE] {
    let mut rec = [0xFF; RECORD_SIZE];
    rec[0..4].copy_from_slice(&MAGIC);
    rec[4] = VERSION;
    rec[5] = tag;
    rec[EXTRA_AT..EXTRA_AT + EXTRA].copy_from_slice(extra);
    rec[CELLS_AT..CHECKSUM_AT].copy_from_slice(cells);
    let sum = checksum(&rec[..CHECKSUM_AT]);
    rec[CHECKSUM_AT..CHECKSUM_AT + 4].copy_from_slice(&sum.to_le_bytes());
    rec
}

fn decode(rec: &[u8], tag: u8) -> Option<([u8; CELLS], [u8; EXTRA])> {
    if rec[0..4] != MAGIC || rec[4] != VERSION || rec[5] != tag {
        return None;
    }
    let stored = u32::from_le_bytes([
        rec[CHECKSUM_AT],
        rec[CHECKSUM_AT + 1],
        rec[CHECKSUM_AT + 2],
        rec[CHECKSUM_AT + 3],
    ]);
    if stored != checksum(&rec[..CHECKSUM_AT]) {
        return None;
    }
    let mut cells = [0xFF; CELLS];
    cells.copy_from_slice(&rec[CELLS_AT..CHECKSUM_AT]);
    let mut extra = [0xFF; EXTRA];
    extra.copy_from_slice(&rec[EXTRA_AT..EXTRA_AT + EXTRA]);
    Some((cells, extra))
}

/// FNV-1a: tiny, and catches partly written records.
fn checksum(bytes: &[u8]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}
