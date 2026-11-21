# Bayleaf Engine

A game engine and editor written from scratch in Rust.

- Native editor (egui on wgpu). The web build compiles the same code to WebAssembly.
- A scene of objects with components, backed by an archetype ECS.
- Game scripts as sandboxed WebAssembly modules with hot reload.
- Node graphs for materials and visual scripting.
- Opt-in gameplay modules and optional world streaming.
- Every action is a command, so the editor, `bayleaf-cli` and AI agents (MCP) drive the same engine.

Status: pre-alpha skeleton. See [docs/vision.md](docs/vision.md).

## Build

```
cargo run -p bayleaf          # editor
cargo run -p bayleaf-cli -- commands
cargo xtask layers
```

Requires Rust 1.95 or newer.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
