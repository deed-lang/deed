//! Hashing a Deed value, in the one place both engines can read.
//!
//! There are two implementations of this walk and there have to be: the
//! interpreter reads a `Value` and the compiled backend reads linear memory,
//! and neither can run the other's code. What must not be written twice is the
//! arithmetic, because a hash is an `Int` a program can print and assert on,
//! so two engines computing it differently is two engines disagreeing about
//! what a program means.
//!
//! So the constants and the step live here, the interpreter folds `Value`
//! through them, and the backend emits the same two instructions with the same
//! two numbers taken from these constants rather than typed again.
//!
//! FNV-1a, because it is four lines and no table, which is what a workspace
//! with no dependencies can hold without ceremony. It is not cryptographic and
//! it is not seeded, and
//! `design/decisions/2026-08-05-a-hash-is-the-equality-walk.md` says what that
//! costs and why there is no seed to be had.

/// Where the fold starts.
pub const BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// What each byte is multiplied by, after being exclusive-ored in.
pub const PRIME: u64 = 0x0000_0100_0000_01b3;

/// A hash being built.
///
/// Absorbing rather than returning a number per part, because the parts of a
/// value are absorbed in order and the order is what tells `[[1], [2]]` from
/// `[[1, 2]]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hash(u64);

impl Default for Hash {
    fn default() -> Self {
        Hash::new()
    }
}

impl Hash {
    pub fn new() -> Hash {
        Hash(BASIS)
    }

    /// Takes in one byte.
    pub fn byte(&mut self, byte: u8) {
        self.0 ^= u64::from(byte);
        self.0 = self.0.wrapping_mul(PRIME);
    }

    /// Takes in one word, whole.
    ///
    /// Not as its eight bytes. A word is what both engines already hold — the
    /// interpreter has an `i64` and the backend has one on the stack — and
    /// folding it whole is one exclusive-or and one multiply on each side
    /// instead of eight, with no agreement needed about which end comes first.
    ///
    /// The multiplier is odd, so multiplying is a bijection on every low bit,
    /// and the exclusive-or moves the low bits before it. Sequential keys
    /// therefore land in different buckets under a mask, which is the case a
    /// map built on this will meet first.
    pub fn word(&mut self, word: i64) {
        self.0 ^= word as u64;
        self.0 = self.0.wrapping_mul(PRIME);
    }

    /// Takes in the characters of a string.
    ///
    /// The bytes of its UTF-8, which is what both engines hold. A string's
    /// length in characters is absorbed by the caller when it absorbs a
    /// length, the same way a list's is.
    pub fn text(&mut self, text: &str) {
        for byte in text.as_bytes() {
            self.byte(*byte);
        }
    }

    /// The number so far, as the `Int` a program sees.
    ///
    /// Reinterpreted rather than clamped, because there is no unsigned type
    /// here and half of every hash would otherwise be the same number.
    pub fn done(self) -> i64 {
        self.0 as i64
    }
}

/// The hash of one word on its own, which is what an `Int` or a `Bool` is.
pub fn of_word(word: i64) -> i64 {
    let mut hash = Hash::new();
    hash.word(word);
    hash.done()
}

/// The hash of a string on its own.
pub fn of_text(text: &str) -> i64 {
    let mut hash = Hash::new();
    hash.text(text);
    hash.done()
}

#[cfg(test)]
mod tests {
    use super::{Hash, of_text, of_word};

    /// The constants are the whole compatibility surface between two engines
    /// and a release, so they are written down twice on purpose: once as the
    /// arithmetic above and once as an answer here. The two string answers are
    /// FNV-1a's own published vectors, so this also says which FNV this is.
    #[test]
    fn the_algorithm_has_an_exact_answer() {
        assert_eq!(Hash::new().done(), -3_750_763_034_362_895_579);
        assert_eq!(of_text(""), -3_750_763_034_362_895_579);
        assert_eq!(of_text("a"), 0xaf63_dc4c_8601_ec8c_u64 as i64);
        assert_eq!(of_text("foobar"), 0x8594_4171_f739_67e8_u64 as i64);
    }

    /// Absorbing in order, so a walk that reads the same parts in a different
    /// order is a different hash rather than a coincidence.
    #[test]
    fn order_is_part_of_the_answer() {
        let mut one = Hash::new();
        one.word(1);
        one.word(2);

        let mut other = Hash::new();
        other.word(2);
        other.word(1);

        assert_ne!(one.done(), other.done());
    }

    /// Sequential keys are what a map meets first, and a hash whose low bits
    /// did not move under them would put all of them in one bucket.
    #[test]
    fn sequential_words_reach_different_buckets() {
        let buckets: Vec<i64> = (0..8).map(|key| of_word(key).rem_euclid(8)).collect();
        let mut seen = buckets.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 8, "eight keys into eight buckets: {buckets:?}");
    }
}
