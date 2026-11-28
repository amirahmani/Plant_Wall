//! Byte format of one saved settings record (the flash access itself is in
//! firmware/src/settings.rs). What the payload bytes mean is in packing.rs.

const MAGIC: [u8; 4] = *b"PLNT";
// Version 5: new payload layout (light in hours, learned pot size), see packing.rs.
// Version 4: plant cells store the water target as a %.
// Version 3: adds the joystick center (4 more bytes).
// Versions 2..4 are still read (missing bytes = 0xFF = not set) and the version
// is returned so packing.rs can convert them. Version 1 (32 bytes) is ignored.
pub const VERSION: u8 = 5;

pub const RECORD_SIZE: usize = 64;

/// Settings bytes per record (between the header and the checksum)
pub const PAYLOAD: usize = 54;
const PAYLOAD_V2: usize = 50; // a version 2 record had 4 bytes less

// Record layout:
//   0..4   magic "PLNT"
//   4      format version
//   5      layout tag (MAX_PLANTS), so a different grid shape isn't misread
//   6..60  payload                         (version 2: 6..56)
//   60..64 checksum of bytes 0..60         (version 2: 56..60)
const PAYLOAD_AT: usize = 6;
const CHECKSUM_AT: usize = PAYLOAD_AT + PAYLOAD;

const _: () = assert!(CHECKSUM_AT + 4 == RECORD_SIZE, "settings record size changed");

pub fn encode(payload: &[u8; PAYLOAD], tag: u8) -> [u8; RECORD_SIZE] {
    let mut rec = [0xFF; RECORD_SIZE];
    rec[0..4].copy_from_slice(&MAGIC);
    rec[4] = VERSION;
    rec[5] = tag;
    rec[PAYLOAD_AT..CHECKSUM_AT].copy_from_slice(payload);
    let sum = checksum(&rec[..CHECKSUM_AT]);
    rec[CHECKSUM_AT..CHECKSUM_AT + 4].copy_from_slice(&sum.to_le_bytes());
    rec
}

/// Returns (payload, version) if `rec` is a valid record for this layout `tag`.
pub fn decode(rec: &[u8], tag: u8) -> Option<([u8; PAYLOAD], u8)> {
    if rec.len() < RECORD_SIZE || rec[0..4] != MAGIC || rec[5] != tag {
        return None;
    }
    let version = rec[4];
    let len = match version {
        3..=VERSION => PAYLOAD,
        2 => PAYLOAD_V2,
        _ => return None,
    };
    let checksum_at = PAYLOAD_AT + len;
    let stored = u32::from_le_bytes([
        rec[checksum_at],
        rec[checksum_at + 1],
        rec[checksum_at + 2],
        rec[checksum_at + 3],
    ]);
    if stored != checksum(&rec[..checksum_at]) {
        return None;
    }
    let mut payload = [0xFF; PAYLOAD]; // bytes an older record doesn't have stay 0xFF = not set
    payload[..len].copy_from_slice(&rec[PAYLOAD_AT..checksum_at]);
    Some((payload, version))
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

    fn sample() -> [u8; PAYLOAD] {
        core::array::from_fn(|i| 100 + i as u8)
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let p = sample();
        assert_eq!(decode(&encode(&p, TAG), TAG), Some((p, VERSION)));
    }

    #[test]
    fn wrong_layout_tag_is_rejected() {
        assert_eq!(decode(&encode(&sample(), TAG), TAG + 1), None);
    }

    #[test]
    fn any_flipped_bit_is_detected() {
        let good = encode(&sample(), TAG);
        for byte in 0..RECORD_SIZE {
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
        for v in [1, VERSION + 1] {
            let mut rec = encode(&sample(), TAG);
            rec[4] = v;
            assert_eq!(decode(&rec, TAG), None, "version {v}");
        }
    }

    /// A record as an older firmware wrote it
    fn old_record(version: u8, len: usize) -> [u8; RECORD_SIZE] {
        let p = sample();
        let mut rec = [0xFF; RECORD_SIZE];
        rec[0..4].copy_from_slice(&MAGIC);
        rec[4] = version;
        rec[5] = TAG;
        rec[PAYLOAD_AT..PAYLOAD_AT + len].copy_from_slice(&p[..len]);
        let sum = checksum(&rec[..PAYLOAD_AT + len]);
        rec[PAYLOAD_AT + len..PAYLOAD_AT + len + 4].copy_from_slice(&sum.to_le_bytes());
        rec
    }

    #[test]
    fn version_4_record_still_loads() {
        let (payload, version) = decode(&old_record(4, PAYLOAD), TAG).expect("v4 record rejected");
        assert_eq!(version, 4);
        assert_eq!(payload, sample());
    }

    #[test]
    fn version_2_record_still_loads() {
        let (payload, version) = decode(&old_record(2, PAYLOAD_V2), TAG).expect("v2 record rejected");
        assert_eq!(version, 2);
        assert_eq!(payload[..PAYLOAD_V2], sample()[..PAYLOAD_V2]);
        assert!(payload[PAYLOAD_V2..].iter().all(|&b| b == 0xFF), "missing bytes must read as not set");
    }
}
