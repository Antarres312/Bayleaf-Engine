# Bayleaf Engine: vision and scope

Status: draft, 2026-11-21. Internal design document. It compares Bayleaf with other engines.
Product copy, labels and help text MUST NOT do that (craft-contrib `standards/naming.md`).

## What Bayleaf is

A game engine and editor written in Rust. It follows the Craft Apps conventions
(storytold/craft-contrib) but is an independent project, not a member of the suite.
If we want to join later, that goes through an RFC in craft-contrib.

- The editor is native egui/eframe on wgpu. The web build is the same Rust compiled to WebAssembly.
- Every editor action is a registered engine command. The GUI, `bayleaf-cli`, the JSON control
  channel and the MCP server all drive the same registry.
- Projects live in local files. There are no cloud round trips.

## Where we follow which model

| Area | Model | Bayleaf decision |
|---|---|---|
| Editor workflow (scene view, hierarchy, inspector, project browser, play mode) | Unity-like | Familiar layout, our own controls, icons and wording (clean-room rule 7) |
| Scene object model | Unity-like | A hierarchy of objects with attached components and prefabs, stored as **archetype ECS** data underneath |
| Game scripting | Our own | **WebAssembly modules** with a Rust guest SDK first and other languages later. Sandboxed, hot-reloadable, and they work in the web build |
| Visual scripting | UE-like | A node graph that compiles to the same script runtime as the WASM modules: one ABI, two front ends |
| Material editor | UE-like | A node graph that compiles to WGSL and shares the graph crate with visual scripting |
| Gameplay framework | UE-like, **modular** | Opt-in roles (rules, player controller, pawn/character, game state, player state). Each role is a separate feature; a project can drop or replace any of them |
| World streaming | UE-like, **optional** | A cell-based streaming crate behind a feature flag. Small or specialised projects do not pay for it |
| Asset pipeline | Mixed | Stable asset ids plus sidecar metadata, import on change, cooked output per platform |

## Principles

1. **Engine first.** The document (project, scenes, assets) is plain data. The command engine changes it,
   and the editor is a thin shell over it.
2. **Opt-in everything above the core.** Gameplay framework, streaming, visual scripting, physics and
   audio are separate crates behind cargo features. The core runtime has to stay usable for a
   tiny 2D game.
3. **One runtime for edit and play.** Play mode runs the same scheduler and script host as the
   standalone player.
4. **Headless parity.** Anything the editor can do, `bayleaf-cli` and the MCP server can do (AA 14).
5. **Clean room.** We learn behaviour only from public documentation, papers and tutorials. We copy no
   code, file layouts, class names or wording from other engines. User-visible feature names get
   a trademark check, recorded in `docs/feature-names.md`.

## Layers

`cargo xtask layers` enforces these. A crate may depend only on crates in lower layers.

| Layer | Crates | Role |
|---|---|---|
| L0 | `core` | ids, handles, errors, math re-exports |
| L1 | `ecs` | archetype storage, queries, scheduler |
| L2 | `scene`, `assets`, `graph` | object hierarchy and prefabs; asset database; node-graph model |
| L3 | `render` | wgpu renderer, material graph to WGSL |
| L4 | `script`, `gameplay`, `streaming` | WASM host and ABI; modular gameplay roles; optional world streaming |
| L5 | `engine` | session, command registry, undo history, play mode |
| L6 | `ui-egui`, `automation` | editor shell; control channel and MCP server |
| L7 | `apps/*` | `bayleaf` (editor), `bayleaf-cli`, `bayleaf-player` |

No crate below L6 may use egui, eframe, winit or rfd.

## Milestones

1. **M0 Skeleton (done):** workspace, layer check, command registry, CLI that lists commands.
2. **M1 Scene core (done, 2026-11-21):** archetype ECS with schedule, object hierarchy, `.bscene` files,
   snapshot undo/redo with merged edit gestures, editor hierarchy and inspector, `bayleaf-cli batch`.
   Still open from M1: prefabs, craft-ui theme tokens, the translation system.
3. **M2 Render (done, 2026-11-21):** `bayleaf-render` with built-in shapes (cube, sphere, plane, quad),
   a metallic/roughness material (GGX), directional lights, unlit transparent sprites, the editor ground
   grid, ray picking, and headless PNG output (`scene.render`). The editor has an orbiting scene view with
   click selection. Still open: shadows, textures and sprite images (wait for the asset pipeline),
   environment lighting, gizmos for moving objects, and drawing camera and light icons in the view.
4. **M3 Scripts (done, 2026-11-21):** WASM host on wasmi (fuel and memory limits, faults isolated per
   script), ABI v1 (`docs/script-abi.md`), Rust SDK (`crates/script-sdk`), example scripts (`scripts/`,
   built by `cargo xtask scripts`), hot reload, play mode (`play.start/stop/pause/step/input/status`)
   that restores the edited scene, a Console panel, and a Project panel (asset browser).
   Scripts sit directly on objects, several per object, each shown as its own inspector card named after
   its module; scripts declare properties with defaults (`bayleaf_describe`), and scenes store only
   overrides (scene format version 2; version 1 files are migrated on load).
   Still open: more input (mouse, gamepad), physics queries.
5. **M4 Automation (done, 2026-11-21):** confined file access for agents (`--automation-read-root`,
   `--automation-write-root`, AA 18), `bayleaf-cli mcp` (MCP over stdio, one tool per command), and an
   editor control channel (`bayleaf --control`, loopback with a token) that the MCP server can bridge to.
   See `docs/automation.md`. Still open: MCP resources (scene tree, renders as images), notifications
   when the scene changes, and restricting the token file's access rules on Windows.
6. **M5 Graphs (done, 2026-11-21):** a shared graph model and editor panel; material graphs that compile
   to WGSL (`.bmaterial`, picked per mesh, live preview while editing, animated with Time); visual
   scripts that compile to WebAssembly with the script ABI (`.bscript`, with properties, branching
   and keys); `graph.*` commands sharing the undo history. Scene format version 3 (`mesh.material`).
   See `docs/graphs.md`. Still open: textures in materials, variables and events beyond start and
   update in visual scripts, comments and groups on the canvas, copy and paste of nodes.
7. **M6 Gameplay framework** as opt-in modules.
8. **M7 Streaming** (optional crate), physics and audio integrations.

## Open questions

- Physics: integrate an allowlisted crate or write our own?
- Scene file format: decided in M1 as pretty JSON (`.bscene`, `format: "bayleaf.scene"`, `version: 1`),
  with objects listed parents first in hierarchy order. Revisit if diffs get too noisy.
- Undo stores whole-scene snapshots. That is fine for small scenes; large scenes will need per-command inverse operations.
- How the guest SDK for languages other than Rust is distributed.
