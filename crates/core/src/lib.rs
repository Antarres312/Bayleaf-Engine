//! L0: ids, handles, errors and math re-exports shared by every Bayleaf crate.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use glam as math;
/// Clocks that also work in a browser, where `std::time::Instant::now` is not available.
pub use web_time as time;

pub mod files;

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Error type for failures in the engine core.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("invalid id: {0}")]
    InvalidId(String),
    #[error("the system random source is unavailable: {0}")]
    RandomUnavailable(String),
}

/// Stable 128-bit id for assets and scene objects. It stays the same across renames and moves.
/// Files store it as 32 lowercase hex digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableId(u128);

impl StableId {
    pub const fn from_u128(value: u128) -> Self {
        Self(value)
    }

    pub const fn as_u128(self) -> u128 {
        self.0
    }

    /// A new random id from the system random source.
    pub fn random() -> Result<Self, CoreError> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| CoreError::RandomUnavailable(e.to_string()))?;
        Ok(Self(u128::from_le_bytes(bytes)))
    }

    /// Parses the 32-digit hex form written by [`StableId::to_hex`].
    pub fn parse_hex(text: &str) -> Result<Self, CoreError> {
        if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CoreError::InvalidId(text.to_owned()));
        }
        u128::from_str_radix(text, 16).map(Self).map_err(|_| CoreError::InvalidId(text.to_owned()))
    }

    pub fn to_hex(self) -> String {
        format!("{:032x}", self.0)
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

impl Serialize for StableId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for StableId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse_hex(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::StableId;

    #[test]
    fn hex_round_trip_keeps_the_id() {
        let id = StableId::from_u128(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
        assert_eq!(StableId::parse_hex(&id.to_hex()).unwrap(), id);
    }

    #[test]
    fn parse_hex_rejects_wrong_length_signs_and_non_hex() {
        assert!(StableId::parse_hex("abc").is_err());
        assert!(StableId::parse_hex(&"z".repeat(32)).is_err());
        assert!(StableId::parse_hex(&format!("+{}", "0".repeat(31))).is_err());
    }

    #[test]
    fn random_ids_differ() {
        assert_ne!(StableId::random().unwrap(), StableId::random().unwrap());
    }
}
