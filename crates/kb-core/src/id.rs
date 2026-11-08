//! Node ids: `n` + a ULID (48-bit millisecond time + 80 random bits, Crockford
//! base32, 26 characters). Ids sort by creation time; renaming a card never renames
//! its file. Ids made in the same millisecond by this process stay increasing.

use std::sync::Mutex;

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

static LAST: Mutex<(u64, u128)> = Mutex::new((0, 0));

/// A new ULID for the given Unix time in milliseconds.
pub fn ulid(ms: u64) -> String {
    let mut buf = [0u8; 10];
    getrandom::getrandom(&mut buf).expect("system randomness");
    let mut rand = buf.iter().fold(0u128, |acc, b| (acc << 8) | *b as u128);
    {
        let mut last = LAST.lock().unwrap();
        if ms == last.0 {
            rand = (last.1 + 1) & ((1u128 << 80) - 1);
        }
        *last = (ms, rand);
    }
    let value = ((ms as u128 & ((1u128 << 48) - 1)) << 80) | rand;
    encode(value)
}

fn encode(mut v: u128) -> String {
    let mut out = [0u8; 26];
    for i in (0..26).rev() {
        out[i] = CROCKFORD[(v & 31) as usize];
        v >>= 5;
    }
    String::from_utf8(out.to_vec()).expect("ascii")
}

/// A node id for a card created at `ms`.
pub fn node_id(ms: i64) -> String {
    format!("n{}", ulid(ms.max(0) as u64))
}

/// Node ids are `n` + 26 Crockford characters (also used as file names).
pub fn valid_node_id(id: &str) -> bool {
    id.len() == 27 && id.starts_with('n') && id[1..].bytes().all(|b| CROCKFORD.contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_and_valid() {
        let a = node_id(1_700_000_000_000);
        let b = node_id(1_700_000_000_000);
        let c = node_id(1_700_000_000_001);
        assert!(valid_node_id(&a));
        assert!(a < b && b < c);
        assert!(!valid_node_id("n../../etc"));
        assert_eq!(encode(0), "00000000000000000000000000");
    }
}
