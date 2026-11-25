//! L3: skeletal animation on top of a model's [`Skeleton`](bayleaf_assets::skeleton::Skeleton).
//!
//! - A clip is anything that poses named parts over time ([`ClipSource`]): a glTF model's own
//!   animations ([`ModelClip`]) or clips made elsewhere, such as on a timeline ([`NamedClip`]). An
//!   object's clips are gathered in [`Clips`], by name.
//! - A [`Motion`] is what plays: a clip, a mix of clips by a number (walk and run by speed), or a
//!   layer that plays one motion on part of the skeleton over another (a wave on the arm while the
//!   legs walk).
//! - A [`Player`] plays motions on an object: it fades one into the next, keeps layers, holds the
//!   parameters that mixes and animation states read, and runs the object's animation states (a
//!   [`Machine`](states::Machine), compiled from a `.banim` graph). Each step gives the clip
//!   events it passed and the root joint's movement.
//!
//! Behaviour follows public descriptions of skeletal animation (linear blend skinning, cross-fades,
//! one-dimensional blends and masked layers); no other engine's code or wording is used.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod clip;
mod motion;
mod player;
pub mod states;

pub use clip::{ClipEvent, ClipSource, Clips, ModelClip, NamedClip, NamedTrack};
pub use motion::{Motion, Number, Parameter, Rig, default_root, pin_root};
pub use player::{Layer, MAX_FADE, MAX_LAYERS, MAX_PARAMETERS, Player, Step};

#[cfg(test)]
mod tests;
