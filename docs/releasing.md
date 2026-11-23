# Branches and versions

## Branches

- `main` holds released versions only. Every commit on it is a release, tagged `vX.Y.Z`.
  Nobody commits on `main` directly.
- `develop` is where work comes together. Changes are made on a short branch from `develop`
  (`feature/<what>` for new work, `fix/<what>` for a bug), checked, and merged back into `develop`.
  Small changes may go to `develop` directly.
- Branches are deleted after they are merged.

## Versions

Versions follow Semantic Versioning (`MAJOR.MINOR.PATCH`). Until 1.0 the major number stays 0:

- **MINOR** goes up for a release with new features or a format change (scene, project or graph
  file versions, the script ABI), for example `0.1.0` → `0.2.0`.
- **PATCH** goes up for a release with fixes only, for example `0.2.0` → `0.2.1`.

The workspace `version` in the root `Cargo.toml` is the version all crates and apps report. It
changes only in a release.

## Making a release

1. On `develop`, run the checks in `AGENTS.md` (tests, clippy, fmt, `cargo xtask layers`,
   `cargo xtask licences`).
2. On a `release/X.Y.Z` branch from `develop`, set the new `version` in the root `Cargo.toml`, run
   `cargo check` so `Cargo.lock` follows, note the release under History below, and commit it as
   `Release X.Y.Z`.
3. Open a pull request of `release/X.Y.Z` into `main`. Once it is merged, merge `main` back into
   `develop`, so `develop` has the release commit too.
4. Tag the merged `main`: `git tag -a vX.Y.Z -m "Bayleaf Engine X.Y.Z"`, and push the tag and
   `develop` (`git push origin develop vX.Y.Z`).

A fix needed in a released version before the next release is made on `fix/<what>` from `main`,
released as a patch version the same way, and then merged into `develop`.

## History

- `v0.3.0`: game interfaces: `.bui` layouts and `.bstyle` style files drawn over the game or standing in
  the world, edited in the Interface panel, with parts, keyboard and gamepad focus, elements that block
  the pointer, and script and visual-script nodes; particle systems (the Particle Emitter, from a mesh's
  surface, with size and colour curves); post-processing (bloom, tone mapping, colour adjustment with a
  lookup image, vignette, depth fog, ambient occlusion, and antialiasing Off, Fast or Fine with
  multisampling on or off); material nodes for a stylised look (lights, view direction, Step, Posterize,
  Remap, custom lighting, outlines) and patterns (Fractal Noise, Voronoi, Waves, Gradient, Random);
  Power, Square Root, Vector Length, Lerp and Inverse Lerp, with math outputs taking their inputs' type;
  lighting assets and scene settings; prefabs edited on their own; graphs in tabs of their own; a
  profiler with the GPU's time; files brought in by dropping them (OBJ, OpenEXR, FLAC; glTF animations
  played by Model Animation; images as colour or data with BC1/BC3 compression; sounds streamed from
  their file); Export Game with a script mode per platform (interpreted, compiled, native) and export
  for another desktop system; a showcase example. Raycast is now Cast Ray, after the feature-name check.
  Formats: scene 23, graph 6, project 9, interface layout 2, post-processing 3, import settings 1;
  older files still open.
- `v0.2.0`: visual scripts gain functions (Function, Return, Call Function), typed properties declared in
  the script's panel with Set Property nodes, and a Negate node; script properties have types
  (number, whole number, true/false, vector, colour, text, object) in the Inspector and the Rust SDK
  (`Script::PROPERTIES` is now `&[Property]`); File > Export Game writes a project as a game folder;
  the graph editor gets box selection, right-drag panning, Delete on nodes, nodes that keep up with
  the pointer and side-by-side pins. Formats: scene 16, graph 5, project 5; older files still open.
- `v0.1.0`: the first numbered version. Milestones M0 to M5 are done, M6 (gameplay framework) and
  M7 (physics, audio, streaming) are started, and the editor is translatable; see `docs/vision.md`.
