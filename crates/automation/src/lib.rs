//! L6: automation for AI agents and scripts: an MCP server over the engine command registry, and
//! the control channel of a running editor. File access is confined to `--automation-read-root`
//! and `--automation-write-root` (AGENTS.md rule 18). See `docs/automation.md`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod control;
pub mod mcp;
