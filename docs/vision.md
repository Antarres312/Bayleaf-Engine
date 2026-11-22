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
   format version 7). Then the translation system: the editor's labels, menus, panels, components and
   graph nodes go through craft-i18n, with catalogs embedded from a craft-i18n checkout and a Window >
   Language choice (left-to-right languages; the engine's own messages stay English). Script
   properties have types: number, whole number, true/false, vector, colour, text and object, each
   with its own Inspector field (scene format version 16). Visual scripts have functions with typed
   inputs and outputs (Function, Return and Call Function nodes; `graph.function.*`; graph version
   4); properties declared in the script's panel, with Set Property nodes and `property_set` in the
   ABI (graph version 5). Later the
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
   keeps the extension. The project file (`.bproject`) is not listed: the File menu opens projects. A double-click on a panel's tab (or Maximise in its menu) fills the dock area with that
   panel; another double-click (or Restore Layout) brings the layout back. Dragging an object from
   the hierarchy onto a folder in the Project panel saves it there as a new prefab (`Crate.bprefab`,
   then `Crate 2.bprefab`) linked to the object.
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
   all start; Text pins and text nodes, and `name_of` (`Object::name`). Then: text, object and colour
   variables (`object_ref` and `object_of`, `Object::keep` in the Rust SDK, keep objects from step to step). Then: whole numbers (a Whole Number pin type and variables, To Whole Number,
   Remainder; counts and indexes are whole). Then: scenes load while playing, in place of the playing one or added to
   it, and added ones unload (`scene_load`, `scene_unload`, Load Scene and Unload Scene nodes,
   `play.scene.*` commands).
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
   format version 9). Then: a Joint component (fixed, hinge, ball, slider) holds a body to another
   object, by name, or to the world, with limits and a motor whose speed scripts can change (Set
   Component; scene format version 13). Audio is started: the `audio` feature (`crates/audio`) plays
   Audio Source components (scene format version 14) and scripts' sounds (`sound_play`,
   `sound_stop`, `sound_play_at`; Play Sound, Stop Sound and Play Sound At nodes; `audio.*`
   commands), decoded with craft-audio and heard through the first camera on the default output
   (cpal). The editor and the player play them out loud; the CLI, agents and tests silently. World
   streaming is started: the `streaming` feature (`crates/streaming`) adds a Streamed Area's scene
   under it, read on worker threads, when a camera comes within its Load Distance, and removes it once
   every camera is past the Keep Margin (scene format version 15; `stream.areas`, `stream.wait`).
   Object > Split into Streamed Areas (`stream.split`) cuts the open scene's top objects into one scene
   file per grid cell, leaving cameras, sun lights, environment, rules and players in place.
9. **M8 Game export and script speed** (started). A project builds into a game people run
   without the editor: the player under the game's name and icon, with its scenes, assets and
   scripts, for Windows, Linux, macOS and the web, built only with the modules the game uses.
   Visual scripts are compiled to WebAssembly at export, not at start. Each project chooses in
   Project Settings how its scripts run, per platform, and the editor shows what each choice gives
   up:
   - **Interpreted** (today, wasmi): starts at once and reloads while playing; slowest.
   - **Compiled to machine code** (the default for exported games): the same WebAssembly modules,
     still sandboxed with the same ABI, limits and faults, compiled by a WebAssembly compiler
     ahead of time at export, or when the module loads where ahead of time is not possible (the
     web runs WebAssembly natively). Close to native speed. The compiler crate must be on the
     craft-contrib licence list; to check first: wasmtime (Apache-2.0).
   - **Native** (opt-in): the script crate is linked into the player at export, with no sandbox.
     Fastest, for code its maker trusts (heavy simulation, say); the export warns that a native
     script can do anything a program can, and a fault in it stops the game. First Rust scripts;
     scripts without their source stay compiled WebAssembly.
   - **Native visual scripts** (later, if the profile shows scripts still weigh on a frame): a
     second back end of the visual script compiler writes each `.bscript` as Rust on the SDK, built
     with the Rust scripts in Native mode, so a whole game can be native. Conditions:
     - Same behaviour as the WebAssembly back end (whole-number division by zero, rounding, text,
       event order, objects that are gone), guarded by a test that runs the same graphs, the
       every-node graph among them, through both back ends and compares what they do.
     - A step guard in generated loops (For Each, loops through links) in place of WebAssembly's
       fuel, so a script that never ends stops with a message instead of freezing the game.
     - The export needs a Rust toolchain; the editor checks for it and says what is missing.
     - A new node kind goes into both back ends; the every-node tests check both.
   The editor's play mode keeps running scripts interpreted or compiled to WebAssembly, so reloading
   while playing works whatever the export uses. Before choosing defaults, a frame profile of the
   demo and a heavier test scene measures how much of a frame scripts take in each mode.
   Started: File > Export Game (`project.export`) writes a folder named after the game with the
   player program under that name and a `game` folder of the project's scenes, assets and scripts
   (visual scripts compiled at export; Rust sources, build output and saved values left behind),
   which the player runs when started without a path; Project Settings > Game chooses the window
   icon (project format 5). Then: the Export Game window (platform, script mode per platform in
   Project Settings, project format 6, folder) shows `project.export.check`: whether this platform
   and mode can export, the main scene, visual scripts that compile, Rust scripts built from their
   latest sources, files scenes name that are missing, and how each script would run; the export
   refuses while it finds an error. Still open: other systems than the one exporting, a player built with
   only the game's modules, the program file's own icon, and the script modes.
10. **M9 Scene settings and lighting** (started: the grid is drawn again, Ctrl+S saves a graph in
    use, the scene heads the hierarchy, scenes name a lighting asset in their settings, scene
    format version 17, Project Settings > Graphics sets shadow quality, project format 7, and
    materials show as pictures in the Project panel and the Inspector).
   - **The scene as the hierarchy's root.** The hierarchy shows one top row named after the
     scene (its file name, or Untitled), holding every top object. It is not an object, so it
     cannot be moved, deleted or copied, and objects can be dropped on it to move them to the top.
   - **The scene in the Inspector.** Selecting that row shows the same thing as selecting the
     scene's file in the Project panel: what the file holds (objects, components, the files it
     uses) and the scene's own settings, edited through `scene.settings.*` commands, so agents can
     change them too.
   - **Lighting as an asset.** A lighting asset (a file of its own, name still to choose and record
     in `docs/feature-names.md`) holds what a scene's light looks like: the sky colours, the light
     around everything, fog, and the sun's direction, colour and strength where the scene has no
     Directional Light of its own. A scene names one in its settings, and several scenes can share
     one, so changing the file changes them all. The Project panel makes new ones, and the
     Inspector edits one when its file is selected or from the scene's settings.
   - **Moving old scenes.** An Environment component in an older scene becomes a lighting asset
     beside the scene, which the scene then names; the scene format version goes up.
   - **Project quality.** Project Settings > Graphics holds what applies to every scene: shadow
     resolution, how many cascades the sun's shadow has and how far they reach, and how many
     lights cast shadows at once. (Done: the editor's views, `scene.render` and the player all
     draw with them.)
   - **Material previews in the Project panel.** A material's tile shows the material on a sphere
     instead of its kind's icon, drawn off screen in the background, kept until the file (or an image
     it uses) changes, and drawn again then; a material that does not compile keeps the icon with a
     warning mark. The same picture heads the Inspector when the file is selected. (Done: pictures
     are drawn two per frame and checked against their files every second.)
   - **The two faster script modes from M8.** Compiled to machine code: exported games run their
     WebAssembly modules through an ahead-of-time compiler (wasmtime, once its licence is checked
     against the craft-contrib list), with the same ABI, fuel and memory limits and faults as the
     interpreter, and the same tests passing in both. (Done: wasmtime is Apache-2.0 WITH
     LLVM-exception, on the list; the player compiles each module when the game starts, since loading
     machine code compiled earlier needs `unsafe`, which the engine allows only in the guest ABI.)
     Native: Rust scripts with their source in the
     project are built into the player at export, with no sandbox; the export says plainly that such
     a script can do anything a program can, and scripts without source stay compiled WebAssembly.
     The Export window's check then reports these modes as available, and Project Settings keeps
     the choice per platform. (Done: the export builds a player with the scripts in from the engine's
     source and cargo; native scripts reach the host through the same host functions, so its checks
     and faults apply; see `docs/script-abi.md`.)
   - **Ctrl+S saves the graph being edited.** While the Graph panel is the one in use (a node, the
     canvas or the script panel has the keyboard, or it was last clicked), Ctrl+S saves the open
     graph (`graph.save`) instead of the scene; a graph never saved asks where, as Save As does.
   - **Fix: the Scene view's grid is not drawn.** The Grid switch is on, but no grid shows (for a
     while now). Find which change hid it and draw it again, with a GPU test that renders the view
     and checks grid lines are there when the switch is on and gone when it is off.
   - **Scripts shape their own Inspector.** A library in the Rust SDK lets a script (and later an
     editor plugin) describe how its properties show: groups and foldouts, labels and hints, ranges
     and sliders, buttons that run a function of the script, lists, read-only values, values shown
     only when another has a value, and messages. The script declares this as data in
     `bayleaf_describe` (no code runs in the editor), the Inspector draws it with its own widgets,
     theme and accessibility, and visual scripts get the same options in their panel. Every control
     it draws changes properties through the usual commands, so agents and undo see it too. Plugins
     that draw freely (their own widgets, not only these options) need a plugin model first: what
     they may touch, how they are sandboxed and loaded; that is decided before this part is built.
     (Done, the data part: `inspector` in `bayleaf_describe`, `Layout` in the SDK, the options in the
     visual script panel, buttons through `play.message`. "Lists" are lists of choices; properties
     holding lists of values need a list property type first. Buttons and notes are for Rust scripts;
     visual scripts set options per property. The plugin model is still to decide.)

11. **M10 Game interface, effects and getting content in** (in progress; see what each item still lacks).
   - **Game interface.** Games build their menus and HUD from a document of their own: a layout
     file (`.bui`) of nested elements (panels, rows and columns, text, images, buttons, toggles,
     sliders, text fields, lists, progress bars) and a style file (`.bstyle`) of rules that pick
     elements by kind, name or class and set their look. Layout is flexible boxes only (direction,
     gap, padding, grow and shrink, alignment, fixed or percentage sizes), so a layout reads the
     same way every time; styles cascade in one plain order (kind, class, name, inline) with no
     selectors beyond that, and every value has a unit written out. An Interface component puts a
     layout on screen (over the game, or on a surface in the world). Scripts and visual scripts
     find elements by name, read and set their text and values, show and hide them, and get
     messages when a button is pressed or a value changes; values can also bind to a property or a
     game state value with no script. An editor panel edits a layout visually (the element tree,
     a live preview at chosen screen sizes, the selected element's style) and writes the same files,
     through `ui.*` commands. The interface is keyboard and gamepad navigable with focus moving
     between elements, text goes through the font stack with right-to-left left for later, and the
     player reads the layout for the platform's accessibility tree. (Done: `bayleaf-interface` with
     `.bui` and `.bstyle` files and the flexible-box layout, the Interface component over the game
     in the Game view and the player, keyboard focus, `pressed:`/`changed:` messages, `ui_set_text`,
     `ui_set_value` and `ui_show`, values bound to the game's state, `ui.*` and `play.ui.*`
     commands, each element in the accessibility tree, and a preview at chosen screen sizes in the
     Inspector; see `docs/interface.md`. The Interface panel edits a layout: the element tree, a
     live preview where a click picks an element, the element's fields and style, through
     `ui.element.*`. Parts: a layout used inside another (a character menu in the main display),
     saved from the panel with Save as Part; scripts and visual scripts build the interface while
     playing (Add and Remove Interface Element, a kind or a part), and visual scripts have the
     interface events and changes. Lists scroll, and scripts read values and text (`ui_value`, `ui_text`, Interface Value
     and Interface Text). Style files are edited rule by rule in the Inspector, and the D-pad moves the focus between
     controls, rows drag in the tree, and a part's texts change where it is used (`texts`).)
   - **Particle systems.** A Particle Emitter component: when and how many particles start (a rate,
     bursts, a shape to start on: point, sphere, cone, box, a mesh's surface), how they live (life
     length, speed, gravity, drag, size and colour over their life as curves and gradients,
     rotation), and how they draw (camera-facing sprites with a texture or a flipbook, stretched
     along their motion, or small meshes; blended or added). Simulated on the CPU first, drawn
     instanced; local or world space, looping or once, and Play, Pause and Restart in the Scene
     view with the emitter selected. Scripts start, stop and burst emitters. A material graph can
     be a particle's material. (Done: the Particle Emitter, `bayleaf-particles`, the Scene view's preview and the script calls; starting on the surface of the object's own mesh (`shape: surface`, its model or its shape); size and colour curves of up to 16 points (`sizeKeys`, `colorKeys`, scene version 22), edited in the Inspector; and Play, Stop and Burst Particles in visual scripts.)
   - **Export for the web, and Linux options.** The Web platform: the player built for
     `wasm32-unknown-unknown` with WebGPU (WebGL2 where it is missing), the game's files packed
     beside it, and an `index.html` that loads it; scripts interpreted, saves in the browser's
     storage, sound after the first click, as browsers require. The Export window's check lists
     what does not work on the web (native scripts, compiled mode, threads). Linux: export from
     Windows for Linux once the player can be built there (or pick a prebuilt Linux player), with
     a `.desktop` file and the game's icon, Wayland and X11 both, and an option for an AppImage.
     (Done: export for another desktop system with a player built there, chosen in the Export
     window; a Linux game gets a `.desktop` file, its icon and `install.sh`. Still to do: the web
     player and its packing, building a Linux player from Windows, and AppImages.)
   - **Post-processing.** A Post-processing asset named by the camera or the scene's settings: tone
     mapping (with exposure), bloom, colour adjustment (contrast, saturation, a lookup texture),
     vignette, fog by depth, ambient occlusion in screen space, and an antialiasing choice beyond
     MSAA. Each effect is on or off with its settings, applied in a fixed order, and the Scene view
     can show the result or the plain picture. Project Settings > Graphics sets which effects the
     game may use at all. (Done: `.bpost` assets, effects in that order, and the colour lookup image
     (`color.lookup`, with Make Neutral Lookup to start one). Antialiasing is Off, Fast or Fine, and
     Project Settings > Graphics turns multisampling (4 samples a pixel) on or off.)
   - **Material nodes for stylised shading (a cel-shaded look).** What is missing: the light's
     direction, colour and shadow at the point (for each light, and the main one), the view
     direction, Dot Product, Normalize, Step, Smoothstep, Lerp, Posterize (bands), Remap, Power,
     Saturate, Split and Combine (vectors), Texture sampled at a computed coordinate (for a ramp
     texture), Screen Position, and a Custom Lighting output that replaces the engine's lighting
     with the graph's own result. An outline option on the material draws the back faces grown
     along their normals in a colour, with a width. An example toon material ships with the
     engine to show them together. (Done: these nodes, the Custom Lighting Output, the outline option and
     `examples/demo/materials/toon.bmaterial`; a graph reads the main light and the others summed.)
   - **Editing prefabs on their own.** Opening a prefab (double-click it, or Open Prefab on an
     instance) swaps the Scene view and the hierarchy to the prefab's own space for a while: only
     its objects, around the origin, with a bar saying which prefab is open and buttons to save it
     and go back. The open scene stays as it was underneath, with its selection and undo history;
     edits to the prefab have their own undo, saving writes the prefab file and every instance in
     the scene updates, keeping its own overrides. Nested prefabs open one inside another, with
     the bar showing the path back. (Done: `prefab.open`, `prefab.save`, `prefab.close`; double-click a prefab, or Open on a copy.)
   - **A basic profiler.** A Profiler panel shows, frame by frame, how long each part took: scripts
     (each script, the slowest first), physics, streaming, sound, extraction and drawing on the
     CPU, and the GPU's time where the adapter can measure it; with the frame rate, draw calls,
     triangles and memory for meshes and images. It records while playing, keeps the last few
     thousand frames as a graph, and pauses on a click to look at one frame. `profile.*` commands
     give the same numbers to agents, and the player can write them to a file. (Done: the Profiler panel, `profile.*` and `bayleaf-player --profile`, and the GPU's own time where the device gives
     timestamps.)
   - **Bringing content in: dragging a file in just works.** Dropping files from the system onto
     the Project panel copies them into the project (into the folder under the pointer), and onto
     the Scene view or the hierarchy also places them: a model becomes an object with its meshes,
     materials made from its textures and its animations kept; an image dropped on an object
     becomes its material's texture; a sound becomes a Sound Source. Each imported file keeps
     import settings beside it (scale and axes for models, compression, mipmaps and colour space for
     images, streaming or loaded for sounds), shown in the Inspector and applied again when the
     file changes. Formats: glTF/GLB first, then OBJ and FBX where a library allows it under the
     licence list; PNG, JPEG, KTX2, EXR for images; WAV, OGG, FLAC, MP3 for sound. A file that
     cannot be read says why, and nothing half-imported stays behind. (Done: `asset.import`, drops
     on the Project panel's folders, the Scene view and the hierarchy, `scene.file.place`,
     `scene.image.apply`, and import settings for a model's scale and up axis and an image's
     mipmaps, in the Inspector. Windows gives no pointer position with a drop until the pointer
     moves, so a drop there waits a moment for one and otherwise goes into the open folder. OBJ
     models with their MTL libraries and images (craft-mesh), OpenEXR images (craft-exr; kept at 8
     bits, so light above 1 clips) and FLAC sounds (craft-audio) are read. Waiting: FBX, which
     craft-mesh reads through craft-zip, whose `zip` brings `constant_time_eq` (CC0-1.0 OR MIT-0 OR
     Apache-2.0; MIT-0 is not on the licence list); KTX2, which no craft-libs crate reads yet. Images
     take a colour space (colours or plain numbers) and BC1 or BC3 block compression (craft-dds).
     Sounds may stream from their file (WAV, AIFF, MP3, Opus and AAC; Ogg Vorbis and FLAC load
     whole, as craft-audio has no streaming decoder for them). A glTF model's animations are kept
     (its nodes and skin) and played by the Model Animation component, posed on the CPU; morph
     targets are left out.)
   - **Fix: material previews draw only the sphere.** The material pictures (Project panel tiles,
     the Inspector, the Graph panel's preview) show the sky behind the sphere. Draw them with a
     transparent background instead, so only the sphere shows over the panel; the sphere is still
     lit by the same sky light. A GPU test checks the corners of a preview are transparent and its
     centre is not. (Done.)
   - **Fix: the Export window grows too wide with Native chosen.** Choosing Native makes the window
     much wider than it opens, so the folder row's Choose… button is out of sight until the window
     is widened by hand. Long texts (the check's findings, the notes on how each script runs, paths)
     wrap to the window's width instead of widening it, and the folder field gives way so the
     button always shows. A UI test chooses each mode at the window's opening size and checks the
     Choose… button lies inside the window. (Done.)
   - **Interfaces in the world.** An Interface can stand in the scene instead of over the screen: a
     flat panel at the object's place, turned with it, with a size in world units and a number of
     interface points per unit (how sharp it is). It is drawn with the scene (lit or unlit, and
     hidden behind what is in front of it), and can always face the camera (a name tag over a
     character, a health bar over an enemy). The pointer reaches it through the scene: a ray from
     the pointer finds the panel, and its buttons, toggles, sliders and On Click elements answer as
     on the screen, with the same messages to the scripts; the keyboard and gamepad reach a world
     panel once it has the focus. The Scene view shows world interfaces where they stand. Commands
     and scripts treat it as any other interface. (Done: `space: world`, `size`, `pointsPerUnit`,
     `faceCamera` (scene version 21), drawn into a picture each frame and shown on its panel in the
     Game view, the Scene view and the player, the pointer reaching it through the scene. A wall in front of a panel keeps the pointer
     from it, and the panel last pressed on has the keyboard and the gamepad. Its elements are in
     the window's accessibility tree where they show, in the Game view and the player.)
   - **Interface elements block the pointer, or let it through.** A click on the interface does not
     also reach the game: while the pointer is over an element that takes it, scripts read no mouse
     button (`mouse_down`, pointer actions).
     Each element says whether it takes the pointer with a style property: `pointer: block` or
     `pointer: pass`. By default buttons, toggles, sliders, text fields, elements with On Click and
     boxes with a background block it; text, images and empty boxes let it through, so a display's
     layout panels never stop the game. Scripts can ask whether the pointer is over the interface.
     (Done: the `pointer` style property, `pointer_over_interface`, and Pointer Position's Over the
     Interface in visual scripts.)
   - **Graphs in tabs of their own.** Each material and visual script opens in a tab of its own
     instead of sharing one Graph panel, so several graphs are open at once and side by side: the
     tab shows the graph's name and whether it has unsaved changes, and closing a tab with changes
     asks first. Double-clicking a graph that is open brings its tab forward. The `graph.*`
     commands work on the graph in use (the one last opened, activated or worked in), `graph.activate`
     picks another and `graph.list` gives the open graphs. Undo goes back through every graph's
     steps where they were made. (Done; a graph's tab can be dragged beside another, and the tab
     bar of a narrow panel scrolls.)

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
