//! A fast hasher for maps keyed by ids and type ids, which the engine looks up many times a step.
//!
//! The standard hasher is built to stand up to chosen keys, at a cost of tens of nanoseconds a
//! lookup. Ids are mostly random already, so this one mixes each word with a folded multiply (the
//! high and low halves of a 128-bit product, xored) keyed by a seed drawn for each map. Keys come
//! from files too, so the seed keeps someone who writes the ids from choosing ones that collide.

use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};

/// A map with [`IdState`] hashing.
pub type IdMap<K, V> = HashMap<K, V, IdState>;
/// A set with [`IdState`] hashing.
pub type IdSet<K> = HashSet<K, IdState>;

const MIX: u64 = 0x9e37_79b9_7f4a_7c15;
const LAST: u64 = 0xd6e8_feb8_6659_fd93;

fn fold(a: u64, b: u64) -> u64 {
    let product = u128::from(a) * u128::from(b);
    (product as u64) ^ ((product >> 64) as u64)
}

/// Builds [`IdHasher`]s with one seed, drawn when the map is made.
#[derive(Clone, Debug)]
pub struct IdState {
    seed: u64,
}

impl Default for IdState {
    fn default() -> Self {
        Self { seed: RandomState::new().hash_one(MIX) }
    }
}

impl BuildHasher for IdState {
    type Hasher = IdHasher;

    fn build_hasher(&self) -> IdHasher {
        IdHasher { acc: self.seed, seed: self.seed }
    }
}

/// See the module documentation.
#[derive(Clone, Debug)]
pub struct IdHasher {
    acc: u64,
    seed: u64,
}

impl Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.write_u64(u64::from_le_bytes(*word));
        }
        let mut last = [0u8; 8];
        last[..rest.len()].copy_from_slice(rest);
        self.write_u64(u64::from_le_bytes(last) ^ ((rest.len() as u64) << 56));
    }

    fn write_u8(&mut self, value: u8) {
        self.write_u64(u64::from(value));
    }

    fn write_u16(&mut self, value: u16) {
        self.write_u64(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.acc = fold(self.acc ^ value, (self.seed ^ MIX) | 1);
    }

    fn write_u128(&mut self, value: u128) {
        self.write_u64(value as u64);
        self.write_u64((value >> 64) as u64);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn finish(&self) -> u64 {
        fold(self.acc, (self.seed ^ LAST) | 1)
    }
}

#[cfg(test)]
mod tests {
    use super::IdState;
    use std::hash::BuildHasher;

    #[test]
    fn ids_alike_in_their_low_bits_still_spread() {
        // Ids that differ only in their high word, as someone choosing collisions might write them:
        // the low bits the map picks a bucket by still differ across them.
        let state = IdState::default();
        let buckets: std::collections::HashSet<u64> = (0..1024u128).map(|i| state.hash_one(i << 64) & 1023).collect();
        assert!(buckets.len() > 512, "only {} of 1024 buckets used", buckets.len());
    }
}
