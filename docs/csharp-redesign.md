# Bayleaf in C#: a redesign

Status: design, 2026-11-26. Asked for by the operator after the comparison of a C# port with the Rust
engine. Stages 0 and 1 are done (see Stages): the C# tree, now at the repository's root, holds the core, the ECS and the
scene with its files; the Rust tree is still the one users run.

## What is decided and what is not

- **Target:** C# 14 on .NET 10, the long-term-support release (supported to November 2028). C# 13
  (.NET 9) is at the end of its support. .NET 11 with C# 15 came out on 10 November 2026 with two
  years of support; its union types would map the engine's Rust enums more closely, so moving to it,
  or to the next long-term release (.NET 12), is a later, separate change.
- **This departs from the Craft rules.** AGENTS.md rule 11 says apps SHOULD be Rust only, and rule 9
  that shared code lives in storytold/craft-libs. Bayleaf is not a member of the suite (`vision.md`), so
  the departure is ours to record; it is recorded here and, when the port starts, in AGENTS.md's
  `## This repository` section. What we lose: craft-ui, craft-fonts-build, craft-markdown, craft-flow,
  craft-audio, craft-audio-dsp, craft-mesh, craft-exr, craft-dds and the suite's egui and winit forks,
  with every fix the suite makes to them later. Code of ours in those libraries may be ported to C#
  (it keeps its notices); nothing else may be copied.
- **What stays exactly as it is:** the principles in `vision.md` (engine first, opt-in modules, one
  runtime for edit and play, headless parity, clean room), the command ids and their JSON parameters
  and results, the automation confinement (rule 18), and every file format (`.bscene`, `.bprefab`,
  `.bscript`, `.banim`, `.btimeline`, `.bshape`, `.bdata`, `.blight`, `.bimport`, `.build` and the
  rest), with the same `VERSION` numbers. A project made in the Rust editor opens in the C# one and
  back, with no conversion.
- **Not decided:** whether the web player stays (see Risks), and whether sandboxed WebAssembly
  scripts stay beside C# scripts (see Scripts). Both are questions for the operator before stage 4.

## Why it can work, and what it costs

The script benchmark showed that the language was never the bottleneck: the same runtime in C++ and
in Rust runs at the same speed, and the gains came from the data layout (`runtime-rebuild.md`). C#
can keep that layout (structs in arrays, `Span<T>`, no allocation per step, NativeAOT for the
player). What it adds is a garbage collector, whose pauses a frame loop must avoid by not allocating,
and bounds checks the compiler does not always remove.

The cost is the size of the engine: about 80,700 lines of Rust (editor UI 21,600; engine 16,200;
script host 14,400 and SDK 3,800; render 5,600; scene 4,700; the rest under 2,100 each) and 1,000
lines of WGSL, which carry over unchanged. Every part is written again; tests are ported as
behaviour, not line by line.

## Layout

One solution, `Bayleaf.slnx` at the root, with one project per layer's part. A project may reference only
projects in lower layers; `tools/Bayleaf.Check` (the `xtask` of the C# tree) checks the references,
the licences and the deny settings, as `cargo xtask layers` and `licences` do now.

The C# tree is the repository's root (`src/`, `tests/`, `tools/`, later `apps/`); on the operator's word of
2026-11-26 the Rust tree moved to `rust/` (its workspace, crates, apps, xtask, scripts and benchmarks), where it
stays buildable and takes fixes until stage 8 removes it. `examples/`, `assets/` and `docs/` are shared.

| Layer | Projects |
|---|---|
| L0 | `Bayleaf.Core`: ids, handles, results, math (`Double3` for positions, since `System.Numerics` has only `float` vectors) |
| L1 | `Bayleaf.Ecs`: archetype storage, queries, schedule, the thread pool option |
| L2 | `Bayleaf.Scene`, `Bayleaf.Assets`, `Bayleaf.Graph` |
| L3 | `Bayleaf.Render` |
| L4 | `Bayleaf.Script`, `Bayleaf.Gameplay`, `Bayleaf.Streaming`, `Bayleaf.Physics`, `Bayleaf.Animation`, `Bayleaf.Audio`, `Bayleaf.Particles`, `Bayleaf.Interface` |
| L5 | `Bayleaf.Engine`: session, command registry, undo, play mode |
| L6 | `Bayleaf.Editor` (the editor's UI), `Bayleaf.Automation` |
| L7 | `apps/Bayleaf` (editor), `apps/Bayleaf.Cli`, `apps/Bayleaf.Player` |

- **Opt-in modules.** A cargo feature becomes its own project, which the engine does not reference;
  an app turns a module on by referencing it and registering it at start (`Engine.Use(new
  PhysicsModule())`). The core builds and runs with none of them, which a test project without them
  checks.
- **Interop.** Native code (wgpu-native, SDL3) is reached only through `Bayleaf.Native`, the one
  project with `AllowUnsafeBlocks`; each `unsafe` block keeps a `// SAFETY:` comment. Every other
  project is safe code.
- **Code rules.** Every project: nullable reference types on, warnings as errors, the .NET analyzers
  at their recommended level, no suppressions (rule 13). Expected failures are results, not
  exceptions; an exception that escapes non-test code is a bug, as a panic is now.
- **Tests.** One test project per project (rule 30), xUnit v3 on Microsoft.Testing.Platform
  (`global.json` picks it; run `dotnet test --solution Bayleaf.slnx`). The editor's
  tests run headless (see Editor), as the egui_kittest tests do now. The test platform reports usage to
  Microsoft unless `TESTINGPLATFORM_TELEMETRY_OPTOUT=1` (or `DOTNET_CLI_TELEMETRY_OPTOUT=1`) is set, as the
  `dotnet` command itself does.

## Data and speed

- Components are structs stored in arrays per archetype; queries hand out `Span<T>`. Nothing in a play
  step allocates once it has warmed up, which a test checks with
  `GC.GetAllocatedBytesForCurrentThread` (the step allocates nothing after the first frames).
- The play runtime keeps what the Rust rebuild reached: packed arrays per object place, one pass to
  fill and read script data, writes through kept keys (`Scene.Key`, `Scene.SetTransform`).
- The editor and play mode run on the JIT, with tiered compilation and dynamic profile-guided
  optimisation. The GC runs in `SustainedLowLatency` during play.
- The exported player: NativeAOT gives one native file, no JIT and a fast start, but stage 0 measured
  it 1.3 times slower than the JIT on the same code, since it compiles without the run's profile. So
  stage 7 measures both for the player: NativeAOT (with a profile collected from a run, if that
  closes the gap) and the JIT with ahead-of-time compiled startup code (ReadyToRun), and picks by
  the numbers.

## Rendering

The renderer keeps its design: `extract` turns a scene into a GPU-independent `Frame`, which is drawn
through WebGPU. The WGSL shaders and the material graph's WGSL output stay as they are.

- WebGPU comes from **wgpu-native** (MIT or Apache-2.0, the C interface of the same wgpu the Rust
  renderer uses), through **Silk.NET.WebGPU** (MIT). Silk.NET's 2.x line is now updated only now and
  then, so the bindings are thin and kept in `Bayleaf.Native`; if they fall behind, we generate our
  own from `webgpu.h`.
- Windows, input and the player's window come from **SDL3** (Zlib).
- `bayleaf-cli run --cmd scene.render` keeps writing a PNG without a window, and GPU tests keep
  skipping without an adapter unless `BAYLEAF_GPU_REQUIRED=1`.

## Editor

- **UI:** Avalonia 12 (MIT). It is the C# toolkit that reaches screen readers on all three desktops:
  UI Automation on Windows, NSAccessibility on macOS and AT-SPI2 on Linux. Standard controls come with
  their roles, names and states; our own drawn surfaces (graph view, keyframes, the scene view's
  gizmos) get automation peers of their own, as AccessKit nodes do now. Keyboard focus is the
  toolkit's focus, not a painted highlight (rule 21).
- **Docking:** dockable, resizable panels with stable widths, as now. A docking library is chosen at
  stage 6 (its licence checked then), or written.
- **Viewport:** the scene and game views are native child windows that draw with WebGPU, hosted in
  Avalonia. Avalonia cannot draw over a native child, so the views' overlays (gizmos, selection,
  labels) are drawn in the WebGPU pass, not as Avalonia controls.
- **Theme:** the tokens in `rust/crates/ui-egui/src/theme.rs` become Avalonia resources, with their
  contrast still tested (4.5:1 for text, 3:1 for controls).
- **Translations:** the craft-i18n catalogs (`common.tsv`, `bayleaf.tsv`) are read as now, through
  the same helpers (`Tl`, `Tc`, `Tid`, `Tf`, `Tn`), English as the key; the pseudo language stays in
  debug builds.
- **Tests:** Avalonia's headless platform runs the panels without a window: click by label, read the
  automation tree, check that keyboard paths have no trap.
- **Panels change state only through commands** (rule 14), as now.

## Scripts

This is the part that changes most, and the part that gains most.

- **C# scripts.** A game's scripts are a C# project against `Bayleaf.Script.Sdk`. The SDK's
  surface is Bayleaf's own and follows `script-abi.md`: properties declared and read into fields when
  they change, one update per object or one update over all of a script's objects (a span of script
  and transform pairs), messages, other objects, data files and the services. Its names are ours; we
  do not take another engine's class or method names.
- **In the editor**, scripts compile with Roslyn (`Microsoft.CodeAnalysis.CSharp`, MIT) into a
  collectible `AssemblyLoadContext`, so a change reloads without restarting; play mode keeps its rules
  (no undo steps, the file not dirtied, the scene restored on stop).
- **In an exported game** the scripts compile into the player with NativeAOT, and their calls into the
  runtime are ordinary calls on the same arrays. That is the Rust rebuild's stage 5, reached without
  a bridge.
- **Visual scripts** compile to C# source, which takes the same path: Roslyn in the editor, NativeAOT
  in a game. NativeAOT has no JIT, so one route that works in both is simpler than an expression-tree
  compiler for the editor and something else for games.
- **The sandbox is lost.** Scripts are WebAssembly modules now: isolated, with fuel and memory limits.
  A C# script runs in the process, and an `AssemblyLoadContext` is not a security boundary. So:
  - opening a project for the first time asks before running its scripts, as a code editor does;
  - limits on run time become a watchdog that stops play mode, not fuel;
  - if sandboxed scripts must stay (mods, scripts from people one does not trust, the Rust scripts
    written so far), `Bayleaf.Script.Wasm` keeps the WebAssembly ABI as an opt-in module, on
    wasmtime's .NET bindings (Apache-2.0 WITH LLVM-exception). This is one of the open questions.

## The other parts

| Part | Today | In C# |
|---|---|---|
| Physics | Rapier | BepuPhysics v2 (Apache-2.0, to be confirmed): multi-threaded already; determinism for the same input and thread count to be checked by test before it is relied on |
| Audio | cpal, craft-audio | SDL3 audio output; our mixer and effects ported from craft-audio-dsp |
| glTF | Rust loader in `rust/crates/assets` | SharpGLTF (MIT, to be confirmed) or a port of ours |
| PNG, JPEG | `png` and others | StbImageSharp (licence to be confirmed); EXR, DDS and OBJ ported from our craft libraries |
| CLI | clap | System.CommandLine (MIT) |
| MCP | `rust/crates/automation` | a port of ours (800 lines); the official C# SDK (MIT) if it saves work |
| JSON | serde | System.Text.Json, with the same field names |
| Animation, particles, game interface, gameplay roles, streaming, graphs | Rust crates | ported as they are |

Not usable, because their licences are not on the Craft list: SixLabors ImageSharp (its own split
licence) and OpenAL Soft (LGPL). Every NuGet package's licence is checked against
`standards/licences.md` before it is added (rule 26); the ones marked "to be confirmed" have not been
yet.

## Stages

Each stage ends with its gate met and the Rust tree still the one users run, until stage 8.

0. **Measure first.** A C# copy of `rust/bench/runtime/rust` beside the C++ one, run on the same scene.
   Gate: the C# step within 1.5 times of Rust's (Rust: 0.24 ms at 20,000 objects, 1.2 to 1.3 ms at
   100,000), with no GC collection during the measured frames. If C# misses it, the port stops here
   and this document records why.
   *Done, 2026-11-26* (`rust/bench/runtime/csharp`, figures in `rust/bench/README.md`, "The same runtime in
   C#"): every checksum equal, no collection, no allocation in the timed steps. On the JIT the script
   step is 1.23 to 1.30 times Rust's and the whole step 1.11 to 1.26: the gate is met. As NativeAOT
   it is 1.60 to 1.63 and 1.36 to 1.59, just over the gate; see the player below. Reading the JIT's
   output found one fix the runtime needed (`Frame.Seconds`), so later stages keep timing their hot
   paths and reading the generated code.
1. **Core, ECS, scene, files.** Every file in the test projects reads and writes back the same as the
   Rust engine reads and writes it.
   *Done, 2026-11-26* (`src/Bayleaf.Core`, `Bayleaf.Ecs`, `Bayleaf.Scene`, with a test project
   each and `tools/Bayleaf.Check`). The files are the scene's own kinds (`.bscene`, `.blight`,
   `.bpost`, `.bshape`, `.bdata`, `.btimeline`); the other kinds belong to the stages that port their
   crates. Each file of `examples/` and `tests/files/` (all 27 component types, every kind of
   script property, version 1 and 16 scenes, duplicate keys, numbers from 1e-7 to 1e30, escapes and
   characters past U+FFFF, and 13 files that must be refused) is read and written back by both engines
   and compared byte for byte with `tests/files/expected/`, which the Rust test writes with
   `BAYLEAF_BLESS=1` and checks otherwise. To give the same bytes, `Bayleaf.Core.Json` writes and reads
   JSON as the Rust engine's library does: the same shortest digits and exponent form (`1e+20`), single
   precision widened to double in untyped values (`0.10000000149011612`), keys sorted by UTF-8 and the
   same rounding when reading decimals. Two differences remain, for a later stage: error messages that
   come from reading JSON are worded differently (the engine's own messages are the same), and the Rust
   reader's leniencies that no file uses (a struct written as an array of its fields) are not copied.
2. **Engine and commands.** Every command id with the same parameters and results, driven by
   `Bayleaf.Cli`. A parity harness runs the same command lists against both CLIs and compares what
   comes back. From here on, new features land in C# only; the Rust tree takes fixes.
   *In progress, 2026-11-26.* `src/Bayleaf.Engine` holds the session, the command registry with its
   undo steps (merged by key, as the editor drags), the automation confinement, and 76 of the Rust
   engine's 172 commands: the scene's files, objects, hierarchy, components and scripts, its settings,
   lighting and post-processing assets, data files and shapes, Input Maps, projects and prefabs (with
   overrides, nesting and editing a prefab on its own) and keyframe animations (editing, the playhead's pose, recording, copying keys; their undo steps share the scene's history), the console and clearing saved values. `apps/Bayleaf.Cli` runs `commands`, `run` and
   `batch` as the Rust CLI does. The other 96 are listed with the Rust engine's id, label, menu,
   shortcut, params and prompt, but disabled with a reason, until their part is ported
   (`src/Bayleaf.Engine/Pending.json` is the Rust engine's list): graphs, play mode, sound, streaming,
   export, the profiler, assets and importing, the game interface, playing timelines, skeletal animation, and
   `scene.render`. The harness: each batch in `tests/commands/` runs in both engines' tests
   (`rust/crates/engine/src/parity_tests.rs`, `tests/Bayleaf.Cli.Tests`), as `bayleaf-cli batch` runs
   it, and what comes back (folders as `$root`/`$out`, ids as `<id N>`) is compared with
   `tests/commands/expected/`, which the Rust test writes with `BAYLEAF_BLESS=1`. All 13 batches agree,
   and every ported command lists the same as in Rust. Running both CLIs as programs comes with the
   automation port, which also brings `mcp`.
3. **Rendering.** `scene.render` images of the test scenes close to the Rust ones (a difference
   threshold, not exact pixels).
4. **Scripts.** C# scripts and visual scripts, hot reload, play mode. Gate: the script-step figures
   of `runtime-rebuild.md` at 20,000 and 100,000 objects, no worse than the Rust engine's at the time.
5. **Physics, audio, animation, particles, game interface, gameplay, streaming.** Their behaviour
   tests ported and passing.
6. **The editor.** Panel by panel, each with its headless tests and its accessibility check.
7. **The player and export.** NativeAOT players for Windows, macOS and Linux; the web decision.
8. **Switch.** The C# editor becomes the release; the Rust tree stays for one release and is then
   removed, with `AGENTS.md`, `vision.md` and the docs rewritten for C#.

## Risks

- **The web player.** .NET in the browser runs on its WebAssembly build of the Mono runtime, with
  ahead-of-time compilation: downloads of several megabytes and slower code than Rust's `wasm32`
  build. Keeping it costs speed and size; dropping it drops a platform `vision.md` promises.
- **Silk.NET's WebGPU bindings** are updated only now and then; owning thin bindings is the fallback.
- **The native viewport inside Avalonia** limits what can be drawn over the scene view to what the
  WebGPU pass draws.
- **GC pauses** if anything in the play step allocates; the allocation test guards it.
- **The suite.** Fixes to craft-libs no longer reach Bayleaf, and a later return to the suite becomes
  a second port.
- **Two trees.** Until stage 8 every fix may be needed in both; stage 2 freezes features in Rust to
  keep that short.

## Questions for the operator

1. Keep the web player, at the cost above, or drop it?
2. Keep sandboxed WebAssembly scripts as an opt-in module beside C# scripts?
3. Stage 1 is done: start stage 2 (the engine, its commands and the parity harness)?
