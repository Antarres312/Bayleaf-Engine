# Script performance benchmark (internal)

This folder times Bayleaf's per-step script work against a plain C++ program that does the same
work, and breaks Bayleaf's time down into its parts. It is an internal engineering tool: its results
and wording are not product copy.

## What runs

**The scene.** A `Target` object first, then N objects on a grid. Each object has a name, a
transform, a mesh (cube), a box collider, a tag, a point light on every tenth object, and three
scripts that all run every step:

| Script | Per step | Host calls, Rust | Host calls, visual |
|---|---|---:|---:|
| spin | rotation Y += speed x dt | 3 | 3 |
| bob | position Y = base + height x sin(time x speed) | 6 | 8 |
| follow | position X and Z ease towards the target's | 5 | 11 |

Property values differ per object by the same formulas in every version. Physics is off (the bench
crate builds `bayleaf-engine` without physics, gameplay, audio or streaming), so colliders and
lights are data only, as they are for the C++ program.

**Versions.** The same scripts as Rust (`bench/scripts/*/src/lib.rs`) and as visual scripts
(`bench/graphs/*.bscript`):

1. Interpreted (wasmi), as the editor runs scripts: Rust and visual.
2. Compiled to machine code (wasmtime), the export mode of that name: Rust and visual.
3. Native: the Rust sources built into the program as a native-mode player builds them
   (`bayleaf_native()` registered with `Session::set_native_scripts` / `ScriptHost::with_natives`).
   They run behind the compiled host's bridge module. Visual scripts have no native form.
4. C++ (`bench/cpp/bench.cpp`), our own code, two layouts: each entity owns its components and its
   scripts are heap objects with a virtual `update` (the ratios use this one), and the same data in
   packed arrays with one loop per script kind.

Control cases that split Bayleaf's time: **no scripts** (the per-step view of the world that the
host builds before any script runs), **3 empty scripts** per object (what calling a script costs
by itself), and **3 empty scripts on the fields path** (the same, in a module that imports the
component functions; see below).

**Parts timed.**

- *Script step*: `ScriptHost::update`, the call play mode makes for scripts each step. For C++,
  the loop over every object's scripts.
- *Whole play step*: the `play.step` command on a headless `Session` with the scene open and play
  started (scripts, requests, particles, sound, streaming, hot-reload checks, console). For C++,
  the script loop plus every object's world matrix, which Bayleaf's step also computes.

## Method

- Release builds only (`lto = "thin"`, `codegen-units = 1`; C++ `cl /O2`). One thread.
- Each case runs in its own process: 30 warm-up steps of 1/60 s, then at least 100 steps and at
  least 3 s, at most 5000 steps (C++: 20000). Each step is timed alone; a run reports mean, median,
  5th and 95th percentile and standard deviation.
- Three runs of each case. Cases on the fields path (visual scripts, native, the `fields`
  control) run three times up to 1000 objects, once at 5000, and not beyond: their loading grows
  with scripts squared (see *Loading*), past four minutes per case at 5000. The tables give the
  median of the runs' means.
- Before each case the runner waits until the processors are less than 15 % busy, and stops a case
  whose private memory passes 12 GB.
- **Loading is not timed, and the script step loads differently from play mode.** `play.start`
  loads every script against the whole scene (see *Loading*), which stops being practical past
  about 1000 objects. For the script step the benchmark loads each instance against a copy of the
  scene that holds only the target (same ids), then times the steps on the whole scene, exactly as
  play mode runs them. At 500 and 1000 objects the runner also times the script step loaded the
  play-mode way (`--full-load`): those steps are 30 to 45 % slower and vary more from run to run
  (see *Reliability*), so the script-step table is a lower bound for play mode. The whole play step
  uses `play.start` as it is, so it runs with scripts only at 500 and 1000 objects.
- Visual scripts are compiled to WebAssembly once before loading, as an export keeps them; the
  editor compiles the graph again for every object that runs it, which only slows loading.
- The Rust scripts are built with a 64 KiB stack (`bench/scripts/.cargo/config.toml`) instead of
  Rust's default 1 MiB, so 60,000 instances fit in memory. This changes memory, not speed.

## Rerun

Needs the `wasm32-unknown-unknown` target and the Visual Studio C++ build tools. From the
repository root, in PowerShell:

```powershell
.\bench\run.ps1
```

It builds the scripts (from `bench/scripts`, so their stack setting applies), the benchmark
(`cargo build --release -p bayleaf-bench -j 2`) and the C++ program, runs every case one after
another (about 2 hours) and writes `bench/results/<date>.jsonl` and `<date>.md`.
`-Objects 500,5000 -Repeats 1` runs fewer cases; `-SkipBuild` reuses the builds; `-SummaryOnly
-Name <date>` rewrites the summary. One case by hand:

```powershell
target\release\bayleaf-bench.exe --mode compiled --scripts graph --part scripts --objects 5000
```

```powershell
target\bench-cpp\bench.exe 5000
```

## Results (2026-11-24)

AMD Ryzen 7 260 (8 cores), 31 GB, Windows 11; Rust 1.100, wasmi 2.0.2, wasmtime 51, MSVC 14.44.
Every case is in [results/2026-11-24.md](results/2026-11-24.md), raw runs in
[results/2026-11-24.jsonl](results/2026-11-24.jsonl). Milliseconds per step (median of the runs'
means); in brackets, times the C++ program whose objects own their components. A blank cell did
not run or did not fit (below).

**Script step** (the update of every script; C++: the loop over every object's scripts)

| Version | 500 | 1000 | 5000 | 10,000 | 20,000 |
|---|---:|---:|---:|---:|---:|
| C++, objects own components | 0.005 (1) | 0.011 (1) | 0.061 (1) | 0.117 (1) | 0.223 (1) |
| C++, packed arrays | 0.004 (0.8) | 0.008 (0.8) | 0.042 (0.7) | 0.105 (0.9) | 0.211 (0.9) |
| Interpreted, Rust | 5.1 (1000) | 12.1 (1140) | 51.5 (845) | 119 (1020) | no fit |
| Compiled, Rust | 4.1 (810) | 10.6 (995) | 47.3 (775) | 118 (1015) | no fit |
| Native, Rust | 18.5 (3650) | 42.8 (4030) | 175 (2870) | | |
| Interpreted, visual | 11.7 (2320) | 26.8 (2520) | 151 (2480) | | |
| Compiled, visual | 11.2 (2210) | 31.9 (3000) | 148 (2430) | | |

**Whole play step** (`play.step`; C++: scripts plus world matrices)

| Version | 500 | 1000 | 5000 | 10,000 | 20,000 |
|---|---:|---:|---:|---:|---:|
| C++, objects own components | 0.012 (1) | 0.024 (1) | 0.132 (1) | 0.295 (1) | 0.656 (1) |
| Bayleaf, no scripts | 1.2 (104) | 2.6 (109) | 13.7 (104) | 32.4 (110) | 71.7 (109) |
| Interpreted, Rust | 9.2 (780) | 24.5 (1010) | | | |
| Compiled, Rust | 7.4 (625) | 21.8 (900) | | | |
| Native, Rust | 22.0 (1860) | no fit | | | |
| Interpreted, visual | 16.3 (1370) | no fit | | | |
| Compiled, visual | 14.9 (1260) | no fit | | | |

**What did not fit.** At 20,000 objects (60,000 script instances) interpreted scripts pass the
12 GB limit (about 270 KB per instance: 8 GB at 10,000 objects), and compiled ones cannot load at
all: wasmtime reserves gigabytes of address space for each instance's memory and runs out ("not
enough memory resources", os error 8). `play.start` with visual scripts or native scripts passes
12 GB at 1000 objects; with Rust scripts it needs 4 GB there.

**The earlier rough numbers** (500 objects: about 4.2 to 4.5 ms per step with a spin script, 1.4 ms
without scripts). The step without scripts matches (1.2 ms). By the split below, one Rust spin
script per object at 500 objects would add about 1 to 1.5 ms, less than was seen then; a visual
spin script anywhere in the scene turns on the fields path (3 to 4 ms more at 500 objects), which
would account for the rest. How those numbers were taken is not recorded, so this is not checked.

### Where Bayleaf's time goes

Splitting the script step with the control cases (interpreted, 5000 objects, 15,000 instances):

| Part | ms per step | per unit |
|---|---:|---:|
| The world view built before any script runs (no scripts) | 10.9 | 2.2 µs per object |
| Calling 3 scripts that do nothing (3 empty scripts, less the view) | 20.3 | 1.35 µs per call |
| The scripts' work and their 14 host calls per object (Rust, less empty) | 20.3 | about 0.3 µs per host call |
| The fields path, when any loaded module imports the component functions | +50 | 10 µs per object |
| Native bridge per host call (native Rust less native empty, at 1000 objects) | | 1.5 to 1.9 µs per host call |

So the WebAssembly the scripts run is a small part. Compiled and interpreted differ by at most 20 %
because almost nothing of the time is spent executing guest instructions; calling into the guest is
slightly dearer compiled. The C++ program spends about 12 ns per object for all three scripts.

- **The world view** (`WorldView::of` in `crates/script/src/lib.rs`) is rebuilt every step for
  every object: its name copied, its world matrix, ray shape, double-precision position, and a list
  of search keys (each component's name and label, tags and script names) as new strings. It is
  most of the no-script step (C++ computes the world matrices alone in about 14 ns per object).
- **Each script call** (`ScriptHost::update` in `crates/script/src/host.rs`): seven `Arc` clones,
  two component lookups for the transform and position, two clocks, two `String` copies of the
  script's name (profiler timings, logs) and a fresh fuel budget, then after the call a component
  write if the transform changed. The profiler timings are kept even when nothing records them.
- **Each host call**: `transform_get` and `transform_set` look the guest's memory export up by
  name and go through a new `Vec`; `property` copies the key into a new `String` and walks a
  `BTreeMap`; `object_of` finds the object by a linear search (the benchmark puts the target first,
  so this is cheap here; a target late in the scene costs a scan of the scene per call).
- **The fields path**: `reads_fields` turns on when any loaded module imports `component_get` or
  `component_set`. Every visual script imports both (its fixed prelude, `crates/script/src/visual.rs`)
  and so does the native bridge module (`crates/script/src/native.rs`). Then every step also
  serialises every readable component of every object to JSON values and copies every script's
  properties. So one visual script, or native mode, slows every script in the scene, and visual
  scripts here cost twice what the same Rust scripts do although they make only 22 host calls per
  object against 14.
- **Native scripts** call the host through `NativeBridge::call`, which formats `call_<name>`, finds
  the export by name, allocates argument and result vectors and makes an untyped call, and finds the
  memory export by name for every read and write. With the fields path on, native scripts are the
  slowest version, 3 to 4 times compiled WebAssembly.
- **The rest of the play step** adds little (0.2 ms at 500 objects without scripts), except the
  hot-reload check: every 0.5 s of play `reload_changed` reads the file time of every instance's
  module, 1500 file-system calls at 500 objects, which shows as steps well above the median.

### Loading

Not timed by the tables, but it decides what can run. `ScriptHost::instantiate` builds the world
view of the whole scene for every instance it loads and the instance keeps it until the first step,
so `play.start` takes time and memory that grow with objects x instances: Rust scripts 3.4 s at 500
objects and 18.6 s and 4 GB at 1000; 2000 objects took 164 s. On the fields path each
instance also copies every loaded script's properties, which grows with instances squared: past
12 GB at 1000 objects, and native mode needed over 70 GB at 5000 before it was stopped. Graphs are
also compiled again for every object that runs them.

### Reliability

- Script step, three runs: the runs' means are mostly within 5 to 10 % of each other; a few runs
  are up to 45 % above the others (interpreted Rust at 5000: 49 to 71 ms), so the median of three is
  given. Within a run the standard deviation is about 8 to 25 % of the mean.
- Cases run once (5000 objects on the fields path) carry about that run-to-run uncertainty.
- Whole step with scripts: the mean is 1 to 2 ms above the median (the hot-reload check), and the
  runs' means differ by up to 2 times (500 objects compiled Rust: 4.7 to 9.1 ms). Loaded as play
  mode loads, the script step is also 30 to 45 % slower than after the quick load. Both come after
  the load's gigabytes of short-lived allocations; the likely cause is where the instances' memory
  ends up (locality), which this benchmark does not prove.
- C++ steps of a few microseconds are near the clock's resolution (0.1 µs) and are given to
  about 10 %. The two C++ layouts are within 10 to 30 % of each other, so the gap is not about how
  the C++ program is laid out.
- Other programs ran on the computer meanwhile; each case waited for the processors to be under
  15 % busy, but short bursts during a case are not excluded.

### What Bayleaf does that the C++ program does not

- Runs each script in its own sandbox: its own WebAssembly instance and memory, with a fuel budget
  set before every call and checked as it runs, and a memory cap.
- Checks every host call: lengths, pointers into guest memory, finite numbers, handles; copies
  values in and out of guest memory instead of sharing pointers.
- Gives every script a consistent view of the scene as the step began (the world view), applies
  transform writes after each call, and supports finding objects by name, kind or kept reference.
- Keeps transforms as scene components in an ECS with stable ids, with validation on every write.
- Collects per-script timings, logs and faults, so a broken script stops alone with a reason.
- Watches module files for hot reload.

The C++ program does none of this: its scripts are trusted code calling each other directly.

## Since these results

The tables above are from before the script host was reworked. After it, on the same computer,
single runs (`target\release\bayleaf-bench.exe`, milliseconds per step, mean):

| Case | Before (develop 1a617f8) | After the world view (a3a359a) | After modules run many objects |
|---|---:|---:|---:|
| No scripts, script step, 5000 | 13.9 | 1.2 | 1.2 |
| Rust, compiled, script step, 5000 | 70.9 | 36.2 | 5.4 |
| Rust, interpreted, script step, 5000 | 77.5 | 40.2 | 15.6 |
| 3 empty Rust scripts, compiled, 5000 | | | 3.1 |
| Rust, compiled, script step, 20,000 | no fit | | 29.4 |
| Rust, compiled, whole play step, 5000 | did not load | | 8.0 median, 16.5 mean |
| Visual, compiled, script step, 1000 | 26.8 | 6.8 | 3.1 |
| Visual, compiled, script step, 5000 | | | 24.0, then 9.0 to 10.8 with the frame |
| Visual, interpreted, script step, 5000 | | | 22.8, then 13.6 with the frame |
| Visual, compiled, script step, 20,000 | no fit | | 120.7, then 58.1 with the frame |

Then at 20,000 objects, with the host's own work per object cut down:

| Case, script step, 20,000 | Before (64c5b13) | Ids hashed fast (faef7cc) | View and transforms kept |
|---|---:|---:|---:|
| No scripts | 7.5 | 4.3 | 0.0 |
| 3 empty Rust scripts, compiled | 26.5 | 15.7 | 5.9 |
| Rust, compiled | 36.7 | 33.9 | 17.3 |
| Visual, compiled | 38.8 | 36.3 | 18.9 |

At 5000 objects after it: Rust 2.9 compiled and 17.0 interpreted, visual 5.3 compiled and 11.1
interpreted. The view of the objects is kept from step to step and only what the scene's log says
moved is read again; scripts' transforms go to one table that later scripts of the step read, and
reach the scene once per object as the step ends; world matrices are worked out only for a ray.

What changed: the world view keeps what changes only with the scene's structure; a Rust script's
module runs every object of its file in one instance with one call per step, its objects'
transforms going in and out in one frame, its properties and the time kept as last read, and the
host selected only before a call that needs it (`docs/script-abi.md`, "Modules that run many
objects"). The whole play step's mean includes reloading's check of module files, now once per file.
Visual scripts run every object of their file in one instance too (each object's variables kept in a slot), and their own transform, properties and the time come from the frame and what each object last read; native scripts still run one call per object. Runs of the same case vary: Rust at 5000 objects measured from 5.4 to 8.0 over the day.

## Cheapest ways to close the gap (not done here)

In rough order of gain for the work:

1. **Turn the fields path on only when used.** Emit `component_get`/`component_set` imports in
   visual scripts only for graphs that use component nodes; do not count the native bridge's
   imports; or gather fields lazily on the first `component_get` of a step. Removes a third to a half of
   visual and native step time and the squared loading.
2. **Share one world view while loading** (build it once per `load_scene`/`load_files`, as
   `update` does) and compile each graph once per file: loading becomes linear.
3. **Build the world view incrementally or lazily**: keep names and search keys across steps and
   rebuild only for objects that changed; compute ray shapes only when a script casts a ray. Most of
   the 2.2 µs per object.
4. **Trim the per-call path**: index the script name instead of copying it twice per call, skip
   timings when the profiler is off, cache the memory export, read and write transforms straight
   from guest memory without a `Vec`, and give `object_of` and `handle_of` a map. Likely a large
   part of the 1.35 µs per call and 0.3 µs per host call.
5. **Fix the native bridge**: resolve each `call_<name>` to a typed function once per script, and
   the memory once. Native scripts should then come in below compiled WebAssembly instead of 3 to 4
   times above it.
6. **Check for reloads by file, not by instance** (each module file once per check), and only in
   the editor, not in exported games.
7. **Fit more instances**: configure wasmtime with a small memory reservation (scripts are capped
   at 64 MiB anyway) and a pooling allocator, and give the script template a small stack, so
   60,000 instances load.

Past these, the remaining gap is the sandbox's own price (a store, fuel and checked calls per
script). Running all scripts of one kind in one instance, or batching host calls (a script reading
and writing its transform through shared memory), would be the larger design changes.

## Other languages, no engine (5000 and 10,000 objects)

`bench/run-languages.ps1` runs the same scene and per-object work in C++ (`bench/cpp`), C#
(`bench/csharp`, C# 5 so Windows' .NET Framework compiler builds it; the same program also runs on
.NET 9 through a runtimeconfig) and Python (`bench/python`, plain CPython, no libraries). Three runs
each; every row in [results/2026-11-24-languages.md](results/2026-11-24-languages.md).

Script step, ms per step (times C++ with objects owning their components):

| Version | 5000 | 10,000 |
|---|---:|---:|
| C++ (MSVC /O2), objects own components | 0.059 (1) | 0.117 (1) |
| C# on .NET 9, objects own components | 0.099 (1.7) | 0.183 (1.6) |
| C# on .NET Framework 4.8, objects own components | 0.272 (4.6) | 0.545 (4.7) |
| Python 3.13, objects own components | 2.2 (38) | 5.8 (50) |
| Bayleaf compiled, Rust scripts (table above) | 47.3 (775) | 118 (1015) |

The runs' means differ by under 2 % for C++ and C#, and by up to 10 % for Python. The Python runs
overlapped with the start of a Bayleaf run on another core.

The Bayleaf numbers in this README were taken on `main` at 0.3.0. The benchmark was then moved onto
`develop`; a quick check there at 500 objects gave somewhat lower step times (interpreted Rust
3.7 ms against 5.1), and the host paths described above are unchanged, but the full suite has not
been run again on `develop`.

## The same runtime in C++ and Rust (2026-11-25)

The question: would the part of the engine that runs a game be faster in C++, or is the design what
counts? `bench/runtime/` holds one small runtime written twice, the same way, in C++
(`cpp/runtime.hpp`) and in Rust (`rust/src/runtime.rs`), running the same game (`cpp/game.cpp`,
`rust/src/game.rs`): this benchmark's scene and its three scripts. The Rust sources also build as one
WebAssembly module, runtime and game together, which `wasm-host/` runs under wasmtime and wasmi.

The runtime is what a fast Bayleaf runtime would be at its core, not a bare loop: components in
packed arrays, objects as an index with a generation that scripts check, objects found by id or name
through hash maps at start, a write to a transform marking the object moved, world matrices worked
out for the moved objects only, and the moved list read as a renderer would read it. Scripts come in
two styles: *typed* (plain structs in one array per script type, one indirect call per type per
step) and *virtual* (a heap object per script and a virtual call per object, as most
entity-and-component code is written). It has no rendering, physics or file loading.

Every version must print the same checksum after 100 steps, and does. Run with
`bench\run-runtime.ps1` (results in `bench/results/2026-11-25-runtime.md`); milliseconds per step,
median of three runs' means:

| Script step | 5000 | 20,000 | 100,000 |
|---|---:|---:|---:|
| C++ (MSVC /O2), typed | 0.061 | 0.244 | 1.30 |
| Rust 1.100, typed | 0.057 | 0.245 | 1.18 |
| C++, virtual | 0.119 | 0.533 | 5.12 |
| Rust, virtual | 0.127 | 0.650 | 10.30 |
| One WebAssembly module, compiled, typed | 0.123 | 0.489 | 2.30 |
| One WebAssembly module, interpreted, typed | 0.870 | 4.51 | 21.2 |
| Bayleaf today (f3ba196), Rust scripts, compiled | 2.9 to 4.2 | 17.3 to 26.0 | 92.4 |
| Bayleaf today, visual scripts, compiled | 4.5 to 5.3 | 18.9 to 26.2 | not run |

| Script step, matrices of what moved, moved list | 5000 | 20,000 | 100,000 |
|---|---:|---:|---:|
| C++ (MSVC /O2), typed | 0.163 | 0.725 | 3.80 |
| Rust 1.100, typed | 0.150 | 0.730 | 3.90 |
| One WebAssembly module, compiled, typed | 0.351 | 1.55 | 6.76 |
| The same, transforms copied in and out each step | 0.279 | 1.54 | 7.64 |

Bayleaf's numbers are two days' runs on the same computer (the range), loading not counted; at
100,000 objects its scene took 320 s to load.

What it says:

- **With the same design, C++ and Rust take the same time**: Rust is at 0.9 to 1.0 times C++ in the
  typed style at every size, and at 1.0 with the matrices.
- **The design is worth 50 to 100 times**: the small runtime runs 20,000 objects in 0.24 ms where
  Bayleaf takes 17 to 26 ms, in either language.
- **How scripts are kept matters more than the language**: a heap object and a virtual call per
  script costs 2 times at 5000 objects and 4 times (C++) to 8 times (Rust) at 100,000, where the
  objects no longer fit the processor's caches. In that style Rust was about 2 times slower than
  C++ at 100,000; its run-to-run spread there was also wide (7.0 to 11.0 ms). The cause was not
  looked into.
- **WebAssembly costs about 2 times** when the runtime and the game are one module with no calls
  out, compiled; interpreted it costs 14 to 18 times. Copying every transform in and out per step
  adds little.

Rebuilding after a change to one script, this small game, release builds (median of three):

| | Seconds |
|---|---:|
| C++: compile `game.cpp` and link (without setting up the compiler's environment, 1.3 s more) | 0.7 |
| Rust, native: `cargo build --release` | 1.8 |
| Rust, the WebAssembly module | 0.7 |
| Bayleaf today: one script's crate to WebAssembly | 0.6 |

These are tiny programs; a real game's rebuild grows with its code in every case.

Limits: one computer; MSVC only (clang was not installed, and Rust's compiler shares clang's code
generator, not MSVC's); a runtime of 300 lines, which shows the cost of the core design and not of
physics, rendering or a real game's scripts.

## The runtime rebuild (from 2026-11-25)

`docs/runtime-rebuild.md` is the plan and says where each stage stands. Its figures come from runs
in turns of the program before and after a change, because single runs on this computer differ by a
third with other work going on. Script step at 20,000 objects, compiled, milliseconds:

| Change | Three empty scripts | Rust scripts | Visual scripts |
|---|---:|---:|---:|
| Before the rebuild (c3d939d) | 9.1 | 20 to 27 | 26 to 28 |
| The SDK serves reads of other objects (8cbfddf) | | 15 to 21 | |
| The frame in one pass, only selected objects looked at (8efed7b) | 2.3 to 3.0 | 7.7 to 7.9 | 22.8 |
| Visual scripts select an object only when needed | | | 7.1 |

Rows were measured on different hours; compare along a row's own before and after (in
`docs/runtime-rebuild.md`), not down a column. The benchmark prints `check`, a sum over the scene;
with `--warmup 30 --min-frames 200 --max-frames 200 --seconds 0` it was the same before and after
each change.
