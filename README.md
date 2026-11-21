# Bayleaf Engine

A game engine and editor written from scratch in Rust.

- Native editor (egui on wgpu). The web build compiles the same code to WebAssembly.
- A scene of objects with components, backed by an archetype ECS.
- Game scripts as sandboxed WebAssembly modules with hot reload.
- Node graphs for materials (compiled to WGSL) and visual scripts (compiled to WebAssembly). See
  [docs/graphs.md](docs/graphs.md).
- Opt-in gameplay modules and optional world streaming.
- Every action is a command, so the editor, `bayleaf-cli` and AI agents (MCP) drive the same engine.

Status: pre-alpha skeleton. See [docs/vision.md](docs/vision.md).

## Build

```
cargo xtask scripts                                 # build the example scripts (once)
cargo run -p bayleaf -- examples/demo/demo.bscene   # editor with the demo; Play, then WASD/Space
cargo run -p bayleaf          # editor
cargo run -p bayleaf -- path/to/Game.bproject   # editor with a project; File > New Project makes one
cargo run -p bayleaf-player -- path/to/Game.bproject   # play the game in its own window (F11: full screen)
cargo run -p bayleaf-cli -- commands
cargo xtask layers
```

Requires Rust 1.95 or newer.

## AI agents

`bayleaf-cli mcp` is an MCP server for AI agents. It runs headless, or drives an open editor started
with `--control`. Agents may only touch the folders you allow:

```
cargo run -p bayleaf-cli -- mcp --automation-read-root examples --automation-write-root out
```

Setup for Claude Code and the editor bridge: [docs/automation.md](docs/automation.md).

## License

Dual-licensed under MIT or Apache-2.0, at your option.
