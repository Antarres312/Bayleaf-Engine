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
2. Set the new `version` in the root `Cargo.toml`, run `cargo check` so `Cargo.lock` follows, and
   commit it on `develop` as `Release X.Y.Z`.
3. Merge `develop` into `main` (`git switch main`, then `git merge --ff-only develop`).
4. Tag it: `git tag -a vX.Y.Z -m "Bayleaf Engine X.Y.Z"`.
5. Push both branches and the tag: `git push origin main develop vX.Y.Z`.

A fix needed in a released version before the next release is made on `fix/<what>` from `main`,
released as a patch version the same way, and then merged into `develop`.

## History

- `v0.1.0`: the first numbered version. Milestones M0 to M5 are done, M6 (gameplay framework) and
  M7 (physics, audio, streaming) are started, and the editor is translatable; see `docs/vision.md`.
