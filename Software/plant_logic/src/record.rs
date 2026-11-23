//! Byte format of one saved settings record (the flash access itself is in
//! firmware/src/settings.rs).

const MAGIC: [u8; 4] = *b"PLNT";
// Version 4: same layout as 3, plant cells store the water target as a % (see packing.rs).
// Version 3: adds the joystick center (4 more extra bytes).
// Versions 2 and 3 are still read (missing bytes = not set), and the version is
// returned so the caller can convert their plant cells. Version 1 (32 bytes) is ignored.
const VERSION: u8 = 4;

pub const RECORD_SIZE: usize = 64;

/// Bytes of plant data per record (one byte per plant).
pub const CELLS: usize = 16;
/// Extra settings bytes per record:
///   0..2   water time hour, minute
///   2..18  calibration LOW (dry) reading per sensor, u16 little-endian, up to 8 sensors
///   18..34 calibration HI (wet) reading per sensor, u16 little-endian, up to 8 sensors
///   34..36 joystick X center, u16 little-endian (version 3)
///   36..38 joystick Y center, u16 little-endian (version 3)
///   0xFF / 0xFFFF = not set
pub const EXTRA: usize = 38;
const EXTRA_V2: usize = 34; // extra bytes in a version 2 record

// Record layout:
//   0..4   magic "PLNT"
//   4      format version
//   5      layout tag (MAX_PLANTS), so a different grid shape isn't misread
//   6..22  plant cells
//   22..60 extra settings (see EXTRA)      (version 2: 22..56)
//   60..64 checksum of bytes 0..60         (version 2: 56..60)
const CELLS_AT: usize = 6;
const EXTRA_AT: usize = CELLS_AT + CELLS;
const CHECKSUM_AT: usize = EXTRA_AT + EXTRA;

const _: () = assert!(CHECKSUM_AT + 4 <= RECORD_SIZE, "settings record too large");

pub fn encode(cells: &[u8; CELLS], extra: &[u8; EXTRA], tag: u8) -> [u8; RECORD_SIZE] {
    let mut rec = [0xFF; RECORD_SIZE];
    rec[0..4].copy_from_slice(&MAGIC);
    rec[4] = VERSION;
    rec[5] = tag;
    rec[CELLS_AT..EXTRA_AT].copy_from_slice(cells);
    rec[EXTRA_AT..CHECKSUM_AT].copy_from_slice(extra);
    let sum = checksum(&rec[..CHECKSUM_AT]);
    rec[CHECKSUM_AT..CHECKSUM_AT + 4].copy_from_slice(&sum.to_le_bytes());
    rec
}

/// Returns (cells, extra, version) if `rec` is a valid record for this layout `tag`.
pub fn decode(rec: &[u8], tag: u8) -> Option<([u8; CELLS], [u8; EXTRA], u8)> {
    if rec[0..4] != MAGIC || rec[5] != tag {
        return None;
    }
    // How many extra bytes this record version has
    let version = rec[4];
    let extra_len = match version {
        3 | 4 => EXTRA,
        2 => EXTRA_V2,
        _ => return None,
    };
    let checksum_at = EXTRA_AT + extra_len;
    let stored = u32::from_le_bytes([
        rec[checksum_at],
        rec[checksum_at + 1],
        rec[checksum_at + 2],
        rec[checksum_at + 3],
    ]);
    if stored != checksum(&rec[..checksum_at]) {
        return None;
    }
    let mut cells = [0xFF; CELLS];
    cells.copy_from_slice(&rec[CELLS_AT..EXTRA_AT]);
    let mut extra = [0xFF; EXTRA]; // bytes an older record doesn't have stay 0xFF = not set
    extra[..extra_len].copy_from_slice(&rec[EXTRA_AT..checksum_at]);
    Some((cells, extra, version))
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

#[cfg(test)]
mod tests {
    use super::*;

    const TAG: u8 = 3;

    fn sample() -> ([u8; CELLS], [u8; EXTRA]) {
        let mut c = [0u8; CELLS];
        let mut e = [0u8; EXTRA];
        for i in 0..CELLS {
            c[i] = i as u8;
        }
        for i in 0..EXTRA {
            e[i] = 100 + i as u8;
        }
        (c, e)
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let (c, e) = sample();
        assert_eq!(decode(&encode(&c, &e, TAG), TAG), Some((c, e, VERSION)));
    }

    #[test]
    fn wrong_layout_tag_is_rejected() {
        let (c, e) = sample();
        assert_eq!(decode(&encode(&c, &e, TAG), TAG + 1), None);
    }

    #[test]
    fn any_flipped_bit_is_detected() {
        let (c, e) = sample();
        let good = encode(&c, &e, TAG);
        for byte in 0..CHECKSUM_AT + 4 {
            for bit in 0..8 {
                let mut bad = good;
                bad[byte] ^= 1 << bit;
                assert_eq!(decode(&bad, TAG), None, "flip byte {byte} bit {bit} not caught");
            }
        }
    }

    #[test]
    fn erased_flash_is_rejected() {
        assert_eq!(decode(&[0xFF; RECORD_SIZE], TAG), None);
    }

    #[test]
    fn unknown_version_is_rejected() {
        let (c, e) = sample();
        let mut rec = encode(&c, &e, TAG);
        rec[4] = 1;
        assert_eq!(decode(&rec, TAG), None);
    }

    #[test]
    fn version_2_record_still_loads() {
        // Build a v2 record by hand: 34 extra bytes, checksum right after them
        let (c, e) = sample();
        let mut rec = [0xFF; RECORD_SIZE];
        rec[0..4].copy_from_slice(&MAGIC);
        rec[4] = 2;
        rec[5] = TAG;
        rec[CELLS_AT..EXTRA_AT].copy_from_slice(&c);
        rec[EXTRA_AT..EXTRA_AT + EXTRA_V2].copy_from_slice(&e[..EXTRA_V2]);
        let sum = checksum(&rec[..EXTRA_AT + EXTRA_V2]);
        rec[EXTRA_AT + EXTRA_V2..EXTRA_AT + EXTRA_V2 + 4].copy_from_slice(&sum.to_le_bytes());

        let (cells, extra, version) = decode(&rec, TAG).expect("v2 record rejected");
        assert_eq!(version, 2);
        assert_eq!(cells, c);
        assert_eq!(extra[..EXTRA_V2], e[..EXTRA_V2]);
        assert!(extra[EXTRA_V2..].iter().all(|&b| b == 0xFF), "missing bytes must read as not set");
    }
}
