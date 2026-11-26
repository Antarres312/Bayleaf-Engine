//! L4: opt-in gameplay roles. Each role is a cargo feature; projects can drop or replace any of them.
//!
//! - `controller`: Player Control: steers an object's Character with input actions and places
//!   the camera behind it.
//! - `rules`: Game Rules: where the player appears and starts over, and when the game ends.
//!
//! Game and player state (named numbers, empty at first) live in the script host, so every
//! project has them; the roles only read and write them through the engine.
//!
//! The roles read the scene and plain numbers (action values); the engine applies what they ask
//! for to physics and the scene, so a project can leave a role out or write its own.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(feature = "controller")]
pub mod controller;
#[cfg(feature = "rules")]
pub mod rules;
