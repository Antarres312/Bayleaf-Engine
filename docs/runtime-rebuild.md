# Rebuilding the play runtime (in Rust)

Status: in progress. Decided 2026-11-25 after the comparison in `rust/bench/README.md` ("The same
runtime in C++ and Rust").

## Why

One small runtime, written the same way in C++ and in Rust, runs this benchmark's scene at the same
speed in both: 0.24 ms per script step for 20,000 objects, 1.2 to 1.3 ms for 100,000. Bayleaf today
takes 17 to 26 ms and 92 ms. The same Rust code as one WebAssembly module takes twice the native
time. So the 50 to 100 times come from how the runtime is built, and Rust reaches them. Moving the
runtime to C++ was considered and not taken: it would write about 32,000 lines again for no gain in
speed, leave the Craft libraries behind and keep two copies of the engine for a long time.

The target is scenes of 100,000 objects.

## What the fast design is

It is what `rust/bench/runtime/rust/src/runtime.rs` does, and what each stage moves Bayleaf towards:

1. During play, transforms and what scripts need of an object live in packed arrays indexed by the
   object's place; nothing is looked up by id per object per step.
2. A step costs what changed. Writes mark an object moved once; world matrices, the scene and the
   renderer take the moved list.
3. A script module gets its objects' data in one block, works on it without calling out, and gives
   it back in one block. A call to the host is for a service (spawn, sound, a ray), not for reading
   or writing what the module could hold.
4. Scripts of one kind sit together and run in one call.

## Stages

Each stage ends with `rust/bench/` numbers at 5000, 20,000 and 100,000 objects in `rust/bench/README.md`, and
keeps every existing test passing.

**Stage 1: Rust scripts make no host call per object.** In a module that runs many objects, the
SDK serves from what it holds: other objects' transforms read once per step, a kept object's handle
until the world's structure changes, the object's own handle from the frame. The SDK's own
bookkeeping per call gets cheaper. Gate: the Rust script step at 20,000 objects under 6 ms.

State: the reads are served (the frame carries each object's handle, `world_stamp` says when
handles change). Run in turns against the SDK before it on the same program, the step at 20,000
objects took 15 to 21 ms against 20 to 27 ms, about a quarter less; the computer was busier than on
the day of the earlier figures. The gate is not met: what is left is mostly the host's work per
object (stage 2) and the SDK's lookups of properties by name (stage 4).

**Stage 2: the host does no work per object.** The frame is filled from and written to the kept
arrays in one pass by index, with no per-object selection, preparation or follow-up; visual scripts
select an object only before a host call that needs it, as Rust scripts do. Gate: three empty
scripts per object at 20,000 objects under 1 ms; Rust and visual script steps under 3 ms.

State: the host half is done. The step sorts its instances only when one comes, goes or stops; a
module's frame is made in one pass, copied out and back as one block, and only the objects the call
selected are looked at afterwards. Run in turns against the host before it, at 20,000 objects:
three empty scripts 9.1 to 2.3-3.0 ms, Rust scripts 14.4 to 7.7-7.9 ms, visual scripts 26-28 to
22.8 ms.

Visual scripts then stopped selecting every object: the frame says which run and whose properties
changed, every import but the frame's own goes through a wrapper that selects the object first, the
object's handle comes from its frame entry, and the object a variable names is kept per object until
`world_stamp` changes. Run in turns, 20,000 objects: 14.8 to 7.1 ms.

The gates are not met yet. On a quiet computer the step at 20,000 objects is about 2 ms for three
empty scripts and 7 to 8 ms for Rust and visual scripts (with other builds running, up to 11). What
is left: the SDK's and the graphs' own work per object (selecting from the frame, loading and saving
a slot, properties looked up by name), which is stage 4, and the frame's bytes written one value at
a time.

**Stage 3: loading.** A scene of 100,000 objects takes 320 s to start playing today. Find what
grows faster than the scene and remove it. Gate: 100,000 objects load in under 10 s.

State: the gate is met. A load now reads and compiles each script file once (a visual script was
compiled again for every object), keeps what it asks of the module, looks through the instances for
one to replace only when the place has one, and finds the module more objects join by its key and
file. Loading is linear in the scene now. With three scripts per object:

| Objects | Scripts loaded as play mode does (`--full-load`) | `play.start` (`--part step`) |
|---:|---:|---:|
| 20,000 | 0.52 s (19.1 s before) | 0.68 s |
| 50,000 | 0.99 s | |
| 100,000 | 1.84 s Rust, 1.69 s visual (320 s before, on the benchmark's own loading) | 2.9 s Rust, 2.5 s visual |

With loading out of the way the script step can be timed at the target size, loaded as play mode
loads: at 100,000 objects, compiled, it takes 8.9 ms for three empty scripts and 37 to 42 ms for
Rust and for visual scripts (92 ms for Rust scripts before the rebuild; other builds were running
during these runs). Stages 4 and 5 are about that figure.

Not part of this stage, but seen here: the first steps after a load are slow (about 200 ms at
100,000 objects, where everything is found and kept for the first time), and the whole play step,
not only scripts, is what stage 6 is about.

**Stage 4: scripts keep their data.** Properties a script reads every step become fields set when
they change, not lookups by name. Typed access to the module's objects' transforms as one slice
(a system over many objects) for the hot cases. Gate: within 3 times of the one-module WebAssembly
figures in `rust/bench/README.md` (about 1.5 ms at 20,000 objects).

State: the SDK half is done. `Script::read_properties` reads a script's properties into its fields
before `init` and whenever they may have changed; `Script::update_many` runs every object of a
script in one go, `Many::iter_mut` giving each one's script and transform straight from the frame,
which the SDK now holds as typed entries. Scripts that do neither run as before, with fewer locks
and no byte decoding. Timing the step in parts showed where the rest went, and two host changes
followed: moved transforms are written back through a key the scene gives for each object
(`Scene::key`, `Scene::set_transform`: 8 ns a write against 27 ns by id), and the frame a module
gives back is read where it is in the module's memory instead of copied out. `rust/bench/scripts/*-many`
are the benchmark's scripts written the new way (`--scripts rust-many`).

Run in turns against the program before this stage, same `check` each time. The computer was busy
with other work during these runs (single runs differed by half), so the ratios are what to read:

| Objects | Three empty scripts | Rust scripts | The same, written for many | Visual scripts |
|---:|---:|---:|---:|---:|
| 20,000 | 0.52 to 0.63 of before | 0.43 to 0.50 | 0.29 to 0.33 | 0.56 to 0.65 |
| 100,000 | 0.74 to 0.78 | 0.41 to 0.58 | 0.30 to 0.33 | 0.64 to 0.67 |

On the quieter morning before, at 20,000 objects: three empty scripts 1.6 to 1.2 ms, Rust scripts
6.9 to 4.0 ms, the same written for many 2.2 ms. The gate is not met yet. What is left, by the
step's parts at 20,000 objects (written for many): filling the frames, 0.33 ms; the modules' calls,
1.1 ms, mostly copying 64 bytes per object per module in and the modules' loops; applying what came
back, 0.38 ms; writing to the scene, 0.4 ms. At 100,000 objects every part is memory bound: three
modules pass about 19 MB of frames each way, and the step grows faster than the scene. Keeping each
module's frame in its memory and writing only what others moved helps only when most objects stand
still; in this benchmark every object moves in every script, so the next gains are fewer bytes per
entry and fewer passes over them (filling the frame in the module's memory directly), and stage 6's
moved list for the scene.

**Stage 5: native scripts run directly.** In an exported game, Rust scripts built into the player
call the runtime as ordinary functions on the same arrays, with no bridge module. Gate: within 2
times of `rust/bench/runtime/rust` (about 0.5 ms at 20,000 objects).

**Stage 6: the rest of the step.** The renderer's extract, physics and the editor's viewport take
the moved list instead of walking the scene, so the whole play step, not only scripts, costs what
moved.

## Done so far

- The scripts' view of the world is kept from step to step; the scene logs transform writes
  (f3ba196).
- Ids are hashed with a seeded folded multiply (faef7cc).
- Modules run every object of their file in one instance with one call per step, transforms going
  in and out in one frame (4da90c9, bf4c6f8, 64c5b13).
- Rust scripts keep their properties in fields and may run all their objects in one update; moved
  transforms reach the scene through kept keys (stage 4).
