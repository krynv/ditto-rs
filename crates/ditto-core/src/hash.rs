//! A small, stable 64-bit FNV-1a hash.
//!
//! Ditto de-duplicates clips with a CRC32 over every clipboard format. We use a 64-bit
//! FNV-1a instead (fewer collisions, stable across Rust releases unlike `DefaultHasher`).
//! The store still compares the full content before treating two clips as equal.

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Clone, Copy, Debug)]
pub struct Fnv64(u64);

impl Default for Fnv64 {
    fn default() -> Self {
        Self(OFFSET)
    }
}

impl Fnv64 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= u64::from(*b);
            self.0 = self.0.wrapping_mul(PRIME);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(Fnv64::new().finish(), 0xcbf29ce484222325);
        let mut h = Fnv64::new();
        h.write(b"a");
        assert_eq!(h.finish(), 0xaf63dc4c8601ec8c);
        let mut h = Fnv64::new();
        h.write(b"foobar");
        assert_eq!(h.finish(), 0x85944171f73967e8);
    }
}
