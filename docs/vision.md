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
   Added later: projects (a folder with a `.bproject` file; `project.new` makes a starting project;
   scene paths start at the project folder), duplicate, copy and paste of objects with their children
   (`scene.object.duplicate/copy/paste`), selecting several objects, and a question before the window
   closes with unsaved work. Prefabs: an object and its children saved as a `.bprefab` file and placed
   as linked copies (`scene.prefab.create/place/apply/revert`); Apply updates every copy in the scene,
   each keeping its own name and transform. Then: each copy keeps the component changes made to it
   (its overrides, listed on its Prefab card) when it is updated from the file; copies update when
   the scene opens; prefabs may hold copies of other prefabs, which follow their own files (scene
   format version 7). Still open from M1: the translation system. Later the
   editor took craft-ui (widget-state colours, sections, combo boxes, the font stack) and with it the
   suite's egui and winit forks; its theme tokens stay its own, as in every Craft app. Then: narrow
   panels put an inspector value under its label instead of past the panel's edge; the Project panel's
   context menus (on files, folders and the space between them) create folders, scenes, materials,
   visual scripts and Rust scripts (`asset.create`), open files with the system's program and show
   them in the file manager; double-click opens a file with its own tool (the editor's, or the
   system's for images, models and text); selected objects are outlined instead of tinted. Then:
   Delete (the key on a focused file, or the context menu) moves a file or folder to the Recycle Bin
   after asking (`asset.delete`; a deleted Rust script leaves the project's workspace); Markdown files
   show formatted in the Inspector (read by craft-markdown), with Source for the text as written; the
   mouse wheel zooms the graph editor; a Rust script is one entry in the Project panel, dragged as its
   module onto an object in the hierarchy, the scene view or the Inspector (built first when it has no
   module yet); File > Open Recent lists the last 10 projects. Then: Add Component lists built-in
   components and scripts in folders by their menu paths (`Light/Point Light`); a script names its
   own (`menu` in `bayleaf_describe`, `Script::MENU`, a visual script's menu path field; graph format 3). Every field that picks a file
   (Material, Model, Image, an Input Map, Player Prefab) also takes a file of its kind dragged from
   the Project panel, and refuses others. Project Settings has Preferences (project format
   4, `settings.editor`): the Project panel hides file extensions unless it is turned off; renaming
   keeps the extension. A double-click on a panel's tab (or Maximise in its menu) fills the dock area with that
   panel; another double-click (or Restore Layout) brings the layout back.
3. **M2 Render (done, 2026-11-21):** `bayleaf-render` with built-in shapes (cube, sphere, plane, quad),
   a metallic/roughness material (GGX), directional lights, unlit transparent sprites, the editor ground
   grid, ray picking, and headless PNG output (`scene.render`). The editor has an orbiting scene view with
   click selection. Added later: move, rotate and scale handles in the view (for every selected object,
   one undo step per drag), clickable camera and light icons, point and spot lights (up to 8 lights,
   windowed inverse-square falloff), shadows from the first directional light (one shadow map fitted to
   the scene, 3x3 filtering), a camera preview in the scene view, and a grid that never flickers with
   surfaces on the ground. Scene format version 4. Then: shadows from spot and point lights
   (`spotLight.shadows`, `pointLight.shadows`; a point light renders six faces; one light casts
   shadows at a time); in scenes wider than 40 units the sun's shadow map is three cascades by
   distance from the camera, so shadows close by stay sharp. Still open: several shadow-casting
   lights at once.
   Added later: PNG images on sprites and in materials (Image and Texture Coordinates nodes, up to 4
   images per material), glTF 2.0 models on meshes (`.glb`, or `.gltf` with its buffers beside it),
   read by `bayleaf-assets` with size limits. Scene format version 5. Then: JPEG images, mipmaps
   averaged as light (with anisotropic filtering), and a model's own base colour, image, metallic and
   roughness from its glTF material (the first material it uses), with its metal/roughness image and
   normal map (no stored tangents needed), and one part per material for models with several. Still
   open: other image formats, skinned and animated models.
   Added later: an Environment component (sky colours above, at and below the horizon, and the light
   they give to shapes from above and below; scene format version 6), and fog in the horizon colour
   (`environment.fogDistance`), and reflections of the sky on surfaces (sharp on smooth ones,
   blurred on rough ones). Still open: reflections of other shapes, light from image files.
4. **M3 Scripts (done, 2026-11-21):** WASM host on wasmi (fuel and memory limits, faults isolated per
   script), ABI v1 (`docs/script-abi.md`), Rust SDK (`crates/script-sdk`), example scripts (`scripts/`,
   built by `cargo xtask scripts`), hot reload, play mode (`play.start/stop/pause/step/input/status`)
   that restores the edited scene, a Console panel, and a Project panel (asset browser).
   Scripts sit directly on objects, several per object, each shown as its own inspector card named after
   its module; scripts declare properties with defaults (`bayleaf_describe`), and scenes store only
   overrides (scene format version 2; version 1 files are migrated on load).
   Added later: the mouse, other objects (find, read and set their transforms), and spawning prefabs
   and destroying objects, in the ABI, the Rust SDK and visual scripts; messages between scripts
   (`send`, delivered after the step to one object or every object); gamepads (the first connected
   one, read with gilrs in the editor and the player); raycasts against mesh shapes and sprites
   (`raycast`, the Raycast node). Physics: see M7. Still open: raycasts against a model's exact shape.
   Scripts in Rust can be made from the Project panel (`script.new`: a crate from a template; in a
   project every script crate joins one Cargo workspace in the project folder, so scripts share a
   Cargo.lock, a build folder and their dependencies, while each still builds to its own module;
   outside a project a script is a workspace of its own) and built to WebAssembly there or with `script.build` (the editor builds in the
   background; agents may not start builds, as build code could reach any file). New files are named
   in place before anything is written; Escape cancels. `cargo xtask licences` checks dependency
   licences and writes NOTICE.
   The Inspector shows a file clicked in the Project panel: what it is and holds (`asset.info`: image
   size, model vertices, scene objects, graph nodes and problems, script properties), an image
   preview, the start of a text file, and the action that fits it. `bayleaf-player` runs a project's
   game in a window of its own, from its first camera, with no editor around it.
5. **M4 Automation (done, 2026-11-21):** confined file access for agents (`--automation-read-root`,
   `--automation-write-root`, AA 18), `bayleaf-cli mcp` (MCP over stdio, one tool per command), and an
   editor control channel (`bayleaf --control`, loopback with a token) that the MCP server can bridge to.
   See `docs/automation.md`. Added later: MCP resources (the scene, a picture of it, the open graph,
   play mode) with subscriptions and change notifications; the token file limited to your account on
   Windows too; an end-to-end test of the MCP bridge to a running session.
6. **M5 Graphs (done, 2026-11-21):** a shared graph model and editor panel; material graphs that compile
   to WGSL (`.bmaterial`, picked per mesh, live preview while editing, animated with Time); visual
   scripts that compile to WebAssembly with the script ABI (`.bscript`, with properties, branching
   and keys); `graph.*` commands sharing the undo history. Scene format version 3 (`mesh.material`).
   See `docs/graphs.md`.
   Added later: selecting several nodes (Ctrl + click), copy, cut, paste and duplicate of nodes with
   the links between them (`graph.node.copy/paste/duplicate`), and zoom keys (Ctrl + Plus, Minus, 0);
   images in materials; comment boxes on the canvas (`graph.comment.*`) that move the nodes inside;
   a material preview on a sphere; variables and On Message / Send Message in visual scripts.
   Then: vector and true/false variables, On Key Pressed, On Timer and gamepad nodes; physics
   contacts reach visual scripts as On Message with `collision`, `collisionEnd`, `triggerEnter` and
   `triggerExit`. Then: a panel beside a visual script's canvas for its variables (declared
   with a type and a starting value, graph format 2), properties and menu path; variable nodes pick
   their variable from a list; hovering a pin, its label or its value shows the type it carries.
   Then: Object pins and searches (This Object, Find Object Named, Find Object With, Count Objects
   With, For Each Object With; `find_with` and `count_with` in the ABI and the Rust SDK) by
   component, tag or script; the actions on other objects take a linked object; Add Node lists
   categories that open on a click. Then: Get Component, Try Get Component and Set Component show the
   chosen component's fields as pins (`component_get`, `component_set`; `Object::field` and
   `Object::set_field` in the Rust SDK). Scripts are components there too: their properties become pins.
   Then: On Init (`bayleaf_init`, `Script::init`): a scene's scripts all load, then all init, then
   all start; Text pins and text nodes, and `name_of` (`Object::name`).
7. **M6 Gameplay framework** as opt-in modules (`gameplay` feature of `bayleaf-engine`). Started:
   input actions in the project settings (named actions bound to keys, mouse, gamepad buttons and
   sticks; `action` in the ABI, Action and On Action Pressed nodes, the Input Actions panel); since
   then Input Map assets (`.binput`, version 2): a list of maps, each with its actions (a trigger,
   held or value, with bindings), of which one map is active at a time, and which code and visual
   scripts name `map/action`; Project Settings names the project's asset (project format 3;
   older projects move their actions into one when opened), and an Input Map component gives an
   object its own; `input_map_set` and the Set Input Map node switch maps, `input.asset`,
   `input.asset.set`, `input.map.set` and `input.state` serve agents, and the Inspector edits an
   asset (scene format 12); and
   the `controller` role: Player Control steers an object's Character with MoveForward, MoveBack,
   MoveLeft, MoveRight and Jump relative to the camera, turns it to face where it walks, and makes the
   first camera trail it (scene format version 10). Then the `rules` role: a Game Rules component
   (scene format version 11) places a player prefab at the first Player Start when the scene has no
   player, sends players that fall below a height back there, and can end the game after a time
   limit, won or lost. Game and player state are named numbers in the script host, empty when play
   starts (`state_get`, `state_set`, `game_end` in the ABI; Game Value, Set Game Value, Change Game
   Value and End Game nodes; `game.state` and `game.state.set` for agents); the ending is announced
   once, in the console and as `gameWon` or `gameLost` to every script. Then: Game Value nodes reach any player (Scope Player
   with a name, or This Object); saved values last between plays in the project's
   `saves/progress.json` (Scope Saved, `saved_value` in the Rust SDK, `game.saved.clear`), written when
   play stops or the game ends, never over a file that did not read; and a Game tab beside the Scene
   tab shows the game through its first camera, takes the game's pointer, and lists the game's,
   players' and saved values. Play brings it to the front and Stop the Scene tab, which stays the
   editor's view during play. Graph node settings can be a list of choices (Scope).
8. **M7 Streaming** (optional crate), physics and audio integrations. Physics is started: the
   `physics` feature of `bayleaf-engine` (`crates/physics`, on Rapier) gives Rigid Body and Collider
   components (box, sphere, capsule; triggers), steps at 60 Hz in play mode, tells scripts about
   collisions and triggers, and lets them push bodies and set or read their velocity (scene format
   version 8). Project settings (`settings` in the `.bproject` file, project format version 2;
   `project.settings`, `project.settings.set`, the Project Settings panel) turn physics off or change
   gravity, and choose single or double precision physics. Then: a Character component (a walking capsule that scripts steer, with steps, slopes,
   sliding and jumps), collider and character outlines in the scene view (a Colliders switch shows
   all), script raycasts against colliders' exact shapes, and `physics.raycast` for agents (scene
   format version 9). Still open: joints.

## Open questions

- Physics: integrate an allowlisted crate or write our own?
- Scene file format: decided in M1 as pretty JSON (`.bscene`, `format: "bayleaf.scene"`, `version: 1`),
  with objects listed parents first in hierarchy order. Revisit if diffs get too noisy.
- Positions are double precision in the scene, world matrices, scripts and the editor's handles; the
  GPU gets single-precision numbers measured from the camera, so worlds far larger than 10 km render
  without jitter. Physics is single precision by default (faster) or double (Project Settings).
- Undo stores whole-scene snapshots, kept as compact JSON within a budget (500 steps, 256 MiB; the
  oldest go first). Very large scenes will still want per-command inverse operations.
- How the guest SDK for languages other than Rust is distributed.
