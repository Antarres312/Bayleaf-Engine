//! Bayleaf Engine player: runs a project's game in a window (see the library).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    bayleaf_player::run(Vec::new())
}

/// In a browser the page starts the player (`bayleaf_player::web`); this program is not built there.
#[cfg(target_arch = "wasm32")]
fn main() {}
