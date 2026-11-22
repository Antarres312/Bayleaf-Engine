//! Bayleaf Engine player: runs a project's game in a window (see the library).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() -> anyhow::Result<()> {
    bayleaf_player::run(Vec::new())
}
