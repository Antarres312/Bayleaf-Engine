# Handing over the C# port

This note is for whoever picks up the port of Bayleaf Engine from Rust to C# 14 on .NET 10. It says
where the work stands, how the pieces fit, how to check a change, and what is left. The design and its
decisions are in [`csharp-redesign.md`](csharp-redesign.md); this note does not repeat them. Where the
two disagree, the design wins and this note should be fixed.

State as of 2026-11-26, `develop` at `db04adb` (pushed to origin).

## Read first

1. [`AGENTS.md`](../AGENTS.md): the rules, and "The C# tree" for this repository's departures.
2. [`csharp-redesign.md`](csharp-redesign.md): the decisions, the layer table, the stages with their
   gates and status notes, and "Questions for the operator".
3. [`script-sdk.md`](script-sdk.md) for C# game scripts, [`automation.md`](automation.md) for the
   commands and MCP, [`graphs.md`](graphs.md) for node graphs.

## Where it stands

| Stage | State |
|---|---|
| 0. Measure | Done. The JIT script step is 1.23–1.30× Rust's. |
| 1. Core, ECS, scene, files | Done. Every shared file is read and written back byte for byte as Rust does. |
| 2. Engine and commands | Done apart from what later stages bring: 162 of the Rust engine's 172 commands are ported. The other 10 are `audio.*` (3), `physics.raycast`, `project.export` and `.check`, and `profile.*` (4). |
| 3. Rendering | Done. `scene.render` pictures match Rust's exactly on the test GPU; extraction matches for every scene. |
| 4. Scripts, play mode | Mostly done. C# scripts (Roslyn, reloading) and play mode work. **Visual scripts do not compile to C# yet**, and the speed gate is met only in part (see the design's stage 4 note). |
| 5. Physics, audio, particles | Not started. The libraries of animation, game interface, gameplay roles and streaming are ported; physics, audio and particles are not. |
| 6. Editor | Not started (Avalonia). |
| 7. Player and export | Not started (NativeAOT players, the web player). |
| 8. Switch | Not started. |

The operator stopped the work after stage 4 to save usage. Ask them before starting more.

## The tree

The C# tree is the repository's root, and the Rust tree is in `rust/`. Users still run Rust until
stage 8. `examples/`, `assets/` and `docs/` are shared.

| C# project | Layer | Ports | Notes |
|---|---|---|---|
| `src/Bayleaf.Native` | L0 | — | The only place for `unsafe`: a safe wrapper over wgpu-native. |
| `src/Bayleaf.Script.Sdk` | L0 | `rust/crates/script-sdk` (in spirit) | What game scripts are written against; names of our own. |
| `src/Bayleaf.Core` | L0 | `crates/core` | Results, ids, geometry (`Double3`, `DMat4`, `DQuat`), JSON written as serde_json writes it, `Files`. |
| `src/Bayleaf.Ecs` | L1 | `crates/ecs` | |
| `src/Bayleaf.Scene` | L2 | `crates/scene` | Components, the scene, its file, timelines, data, lighting. `IPlayModule` lives here. |
| `src/Bayleaf.Assets` | L2 | `crates/assets` | Our own PNG, JPEG, EXR, glTF and OBJ readers; import settings. |
| `src/Bayleaf.Graph` | L2 | `crates/graph` | The node graph model; each domain's catalog lives with its compiler. |
| `src/Bayleaf.Interface` | L2 | `crates/interface` | Namespace `Bayleaf.GameInterface`. |
| `src/Bayleaf.Render` | L3 | `crates/render` | Material catalog and WGSL compiler, extraction, the wgpu renderer. The shaders are linked from `rust/crates/render/src/shaders/`, so there is one copy; move them here in stage 8. |
| `src/Bayleaf.Animation` | L3 | `crates/animation` | |
| `src/Bayleaf.Script` | L4 | `crates/script` | Visual script catalog, the Roslyn host, the watchdog. |
| `src/Bayleaf.Gameplay`, `src/Bayleaf.Streaming` | L4 | `crates/gameplay`, `crates/streaming` | Opt-in modules, plugged into play mode through `IPlayModule`. |
| `src/Bayleaf.Engine` | L5 | `crates/engine` | Session, command registry, undo, play mode, every command (`*Commands.cs`). |
| `src/Bayleaf.Automation` | L6 | `crates/automation` | The MCP server and the editor's control channel. |
| `apps/Bayleaf.Cli` | L7 | `apps/bayleaf-cli` | `commands`, `run`, `batch`, `mcp`. |
| `tools/Bayleaf.Check` | — | `xtask` | `layers` and `licences`. A new project goes into its layer table in the same change. |

Each `src` project has a `tests/<Project>.Tests` with xUnit v3 on Microsoft.Testing.Platform
(`global.json`). Example scripts are in `scripts/`, and the stage 4 benchmark is in `bench/scripts`.

## Checking a change

Run these from the repository's root:

```powershell
dotnet build Bayleaf.slnx -m:2
dotnet test --solution Bayleaf.slnx --no-build
dotnet run --project tools/Bayleaf.Check --no-build -- layers
dotnet run --project tools/Bayleaf.Check --no-build -- licences
```

- **Gentle builds.** A full parallel build once crashed the operator's PC. Use `-m:2` (or build one
  project), and in Rust test one crate with `-j 2`/`-j 4` and `--test-threads=2`/`4`. Never build the
  whole Rust workspace.
- **Rust commands** run in `rust/`, with `CARGO_TARGET_DIR=..\target` to reuse the build cache.
- **GPU tests** skip without an adapter. Set `BAYLEAF_GPU_REQUIRED=1` to make them fail instead; the
  test machine has a GPU.
- **Telemetry:** set `TESTINGPLATFORM_TELEMETRY_OPTOUT=1`.
- **One test:** a test project's exe runs one class or method, for example
  `target\dotnet\bin\Bayleaf.Cli.Tests\debug\Bayleaf.Cli.Tests.exe -class Bayleaf.Cli.Tests.McpCliTests`.
- **Warnings are errors** (`Directory.Build.props`), with the recommended analyzers. Never suppress
  one; rename or restructure.

## Parity: how the two engines are compared

The Rust engine is the reference. Its tests write what it gives into expected files when
`BAYLEAF_BLESS=1` is set, and the C# tests compare with them. Bless in Rust only, never from C#.

| What | Expected files | Bless (in `rust/`) |
|---|---|---|
| Scene, lighting, data, timeline files | `tests/files/expected/<path parts joined by __>` | `cargo test -p bayleaf-scene` |
| Interface layouts and styles | same folder | `-p bayleaf-interface` |
| `.banim` files and what they compile to (`*.summary`) | same folder | `-p bayleaf-animation` |
| `.bmaterial` files, their WGSL (`*.wgsl`), the material catalog | same folder | `-p bayleaf-render parity_tests` |
| `.bscript` files, the script catalog | same folder | `-p bayleaf-script parity_tests` |
| Extraction (`*.bscene.frame.json`) | same folder | `-p bayleaf-render parity_tests` |
| `scene.render` pictures | `tests/files/expected/renders/` | `-p bayleaf-engine --lib parity_tests` |
| Command batches | `tests/commands/expected/` | `-p bayleaf-engine --lib parity_tests` |

`cargo test -p bayleaf-render` builds wgpu, which takes about 5 minutes from a cold cache.

**Command batches** (`tests/commands/*.json`): each is a list of `{cmd, params}` steps run in a fresh
session, as `bayleaf-cli batch` runs them.
- **Placeholders:** `$root` is the repository's folder, `$out` a new empty folder, and `$N.field` a
  field of step N's result.
- **Normalising:** both harnesses turn folders back into `$root`/`$out` and ids into `<id N>`, and cut
  serde's params errors to their first three words.
- **Naming:** a batch that should fail is named `refused-*.json` and ends at its failing step.
- **Known differences** are listed, each with why, in `tests/commands/differences/known.json`. The C#
  test checks C#'s value there and compares the rest.
- **Command listing:** `listing.json` holds where C#'s command list differs on purpose.
- **Pending commands:** `src/Bayleaf.Engine/Pending.json` is the Rust engine's command list. A command
  not ported yet is listed from it, disabled with a reason. A ported spec of the same id takes its place
  automatically.

**A batch must give the same bytes on every machine.**
- Use only files under `tests/files` and files the engines write.
- Avoid errors whose text comes from the operating system, which is worded in its language.
- Avoid errors from third-party decoders, whose wording is the Rust crates'.
- Avoid paths that only one system writes. Known flaw: `refused-import-blocks` has a `\` that only
  Windows writes, because the harness only turns `\` into `/` in strings that start with `$root` or
  `$out`.

## Porting a command

1. Read its Rust file in `rust/crates/engine/src/` and the Rust tests for it in `tests.rs`.
2. Write it in the matching `src/Bayleaf.Engine/*Commands.cs` (or a new one, registered in
   `Commands.cs`).
   - Keep the id, label, menu, shortcut, params text and the Rust engine's English messages.
   - Read params with `Params.Of(p, ...)` and fail with `Stop.BadParams`/`Stop.Failed`.
   - Check every file path with `session.Allowed(path, Access.Read|Write)`.
3. Add batches to `tests/commands/` (one that works, `refused-*` ones for its errors), bless them in
   Rust, and run the C# harness.
4. Port the Rust behaviour tests to `tests/Bayleaf.Engine.Tests`. A bug fix gets a test that fails
   without it (AGENTS rule 29).
5. Update the stage 2 counts and pending list in the design.

## Pitfalls met so far

- **Analyzer names.** CA1716 and CA1720 refuse names that are keywords or type names in .NET
  languages: `Object`, `String`, `Step`, `Error` as an interface parameter, `And`/`Or`/`Not`. The Rust
  names were renamed; the port notes say how.
- **JSON output.** serde_json sorts object keys. Call `.Canonical()` before writing a reply that Rust
  builds with `json!`. An f32 is widened to f64 inside untyped values, which `Canonical` does too.
- **Numbers as text.** Rust's `{:?}` for floats is `RustFloat.Debug` in `Bayleaf.Render`. `{:.2}` is
  the helper in `AssetInfoCommands.cs` (exact value, ties to even).
- **Paths.** `Path.GetDirectoryName` and `Path.GetPathRoot` turn `/` into `\` on Windows, where Rust's
  `Path::parent` keeps the text. Use `Files.Parent` where the text shows in a result or message.
- **Raw string literals** take the line ends the source file was checked out with. Write file text
  with `.ReplaceLineEndings("\n")`, as `Starter.cs` does.
- **Shell scripts.** In this environment, backslash escapes inside heredocs passed to Python were
  turned into real characters (`\r` became a carriage return). Use the Edit tool, or a script file,
  for text with backslashes.
- **Slerp.** Interpolated joint turns differ from Rust in the last f32 digit. glam uses an approximate
  arc cosine and sine; `FMath.Slerp` does not. Batches read joints only where no turn is interpolated.
  Matching glam would be its own change in `Bayleaf.Assets`.

## Differences kept on purpose

These are recorded in the design and in `known.json`:
- no Recycle Bin, so `asset.delete` needs `permanent: true`;
- a visual script's problems say it is not compiled;
- `.wasm` modules are not described;
- `play.start` reports the runtime "compiled";
- JSON reader errors are worded differently;
- JPEG pixels are within 3 per channel of Rust's;
- EXR files with PIZ, B44 or DWA compression are not read yet;
- `CompiledMaterial.Key` is FNV-1a, not SipHash;
- GPU frame times are not measured (this wgpu-native cannot say how long a timestamp tick is).

## What is left

In the order the design gives:

1. **Visual scripts to C#** (the rest of stage 4).
   - Compile a `.bscript` graph (catalog in `src/Bayleaf.Script`) to C# source against
     `Bayleaf.Script.Sdk`, and run it through the same Roslyn host. Rust's compiler to WebAssembly is
     in `rust/crates/script/src/visual.rs` and shows what each node does.
   - Then the problems of a visual script come from this compiler, and the entries in `known.json`
     about "not compiled" go.
   - Until then, a new project's starter cube (a visual script) logs an error in play mode instead of
     spinning.
2. **The stage 4 gate, measured fairly.** Run `bench/scripts` and the Rust figures in turns on a quiet
   machine. Today scripts that move their objects measure about level with Rust to 25–45% slower.
3. **Stage 5.**
   - Physics on BepuPhysics v2, with the determinism test the design asks for.
   - Audio: SDL3 output, our mixer and effects from craft-libs. Only WAV and AIFF can be decoded
     without new packages.
   - Particles.
   - Their pending commands, and their hookup in play mode through the seams it leaves; a call into
     them now reports nothing and logs once.
4. **Stage 6**, the editor on Avalonia, panel by panel with headless tests and the accessibility
   checks of AGENTS rule 21. Also the consent dialog for running a project's scripts, remembered per
   folder.
5. **Stage 7**: NativeAOT players, export (`project.export`), and the web player (kept, through the
   `wasm-tools` workload). Also the profiler commands.
6. **Stage 8**: the switch. Move the shaders into `src/Bayleaf.Render`, remove `rust/`, and rewrite
   `AGENTS.md`, `vision.md` and the docs for C#.

Smaller follow-ups:
- `--allow-scripts` for `bayleaf-cli mcp`.
- Lists and maps as script properties, component event subscriptions, and Inspector groups, buttons
  and notes for scripts.
- The `\` flaw in the harness noted above.
- The bundled wgpu-native is from early 2024 (Silk.NET 2.23). Newer WebGPU features need our own
  bindings; the design names that as the fallback.

## Working with the operator

- **Replies.** The operator writes in Polish; answer in Polish. Code, commits and docs are in English.
- **Pushing** needs the operator's "push" every time. So do merging into `main`, tags and deleting
  remote branches.
- **Packages.** These downloads are approved: Silk.NET.WebGPU with wgpu-native, Roslyn, BepuPhysics2,
  SDL3 bindings, Avalonia with Avalonia.Headless, the NativeAOT compiler, and the `wasm-tools`
  workload. Anything else, including audio decoders other than our own, needs asking first. Every
  package's licence must be on the craft-contrib list; record it in `ATTRIBUTION.md` and run
  `licences`.
- **Questions waiting for the operator:** the script host's choices listed under "Questions for the
  operator" in the design.
- **Branches.**
  - Work on `feature/<what>` or `fix/<what>` from `develop`, then fast-forward `develop` onto it and
    delete the branch.
  - Commit with `git add` of explicit files, never `.claude/`.
  - End messages with the co-author line the earlier commits use.
- **Agent worktrees** start on `main`, which has no C# tree. Switch to a branch from `develop`'s commit
  first.
  - Old port worktrees under `.claude/worktrees/` (`port/*` branches) are merged and can be removed
    with the operator's OK.
  - Merges between parallel ports usually conflict only in `Bayleaf.slnx`, `ATTRIBUTION.md` and the
    stage paragraphs of the design.
- **Usage.** The operator asked to spend less of the usage limits: prefer one agent at a time, with a
  narrow brief.
