//! A hash for keys that are already numbers.
//!
//! Everything this compiler keys a map by is a small integer or two: a
//! [`Span`](crate::Span) is a pair of byte offsets, a definition is an index
//! into a table. The standard library hashes with SipHash, which is chosen to
//! survive an attacker choosing the keys. Nothing here has that problem: the
//! keys come out of a file the compiler was handed, and a file that could pick
//! its own spans could do far worse than make a hash map slow.
//!
//! Measured before it was written, because that is the rule. Reading a name
//! was the most expensive small thing in the language and it is two lookups,
//! one from the span where the name is written to what it refers to and one
//! from that to the value. `crates/deed-driver/examples/interpreting.rs` says
//! what a name costs and what it costs now.
//!
//! The mixing is one multiply and one shift, the same shape as the hash used
//! by every compiler that has had this problem, and it is written out here
//! rather than depended on because it is nine lines.

use std::hash::{BuildHasherDefault, Hasher};

/// A [`BuildHasher`](std::hash::BuildHasher) for maps keyed by numbers.
pub type ByNumber = BuildHasherDefault<NumberHasher>;

/// Mixes whatever it is given into one word.
///
/// The constant is the 64 bit odd number used for this everywhere: multiplying
/// by an odd number is reversible, so no two inputs collide before the shift,
/// and the shift moves the high bits, which the multiply mixed best, down to
/// where a hash map reads them.
#[derive(Default)]
pub struct NumberHasher(u64);

const MIX: u64 = 0x517c_c1b7_2722_0a95;

impl NumberHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0 ^ word).wrapping_mul(MIX);
    }
}

impl Hasher for NumberHasher {
    fn finish(&self) -> u64 {
        // The multiply mixes upward, so the answer lives in the high bits.
        self.0 ^ (self.0 >> 32)
    }

    /// Bytes, for a key that is not a number after all.
    ///
    /// Kept correct rather than fast: `Hasher` is one trait and a map keyed by
    /// something else would otherwise get a hash that ignores most of it. Slow
    /// and right beats fast and wrong, and nothing this is used for takes this
    /// path.
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::hash::Hash;

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = NumberHasher::default();
        value.hash(&mut hasher);
        hasher.finish()
    }

    /// The thing a hash has to do. Two keys that differ in either half have to
    /// be able to tell each other apart, and a hash that ignored the second
    /// word would pass every other test in this file.
    #[test]
    fn two_keys_that_differ_hash_differently() {
        assert_ne!(hash_of(&(1u32, 2u32)), hash_of(&(2u32, 1u32)));
        assert_ne!(hash_of(&(0u32, 1u32)), hash_of(&(0u32, 2u32)));
        assert_ne!(hash_of(&(1u32, 0u32)), hash_of(&(2u32, 0u32)));
    }

    /// And the thing a hash map needs on top of that: the same key twice is
    /// the same entry, whichever map it is in.
    #[test]
    fn a_map_keyed_by_it_still_behaves_like_a_map() {
        let mut map: HashMap<(u32, u32), &str, ByNumber> = HashMap::default();
        for start in 0..64u32 {
            map.insert((start, start + 3), "here");
        }

        assert_eq!(map.len(), 64);
        assert_eq!(map.get(&(7, 10)), Some(&"here"));
        assert_eq!(map.get(&(7, 11)), None);
    }

    /// Bytes go through the slow path, which has to be a hash rather than a
    /// constant, or a map keyed by a string would put everything in one bucket.
    #[test]
    fn a_key_that_is_not_a_number_is_still_hashed() {
        assert_ne!(hash_of(&"one"), hash_of(&"two"));
        assert_ne!(hash_of(&"a longer key than eight bytes"), hash_of(&"one"));
    }
}
