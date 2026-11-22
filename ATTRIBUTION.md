# Attribution

Every third-party asset, vendored file or ported code is listed here, in the same change that adds it.
Third-party Rust crates are listed with their licences in `NOTICE`, written by
`cargo xtask licences --write`, which also checks them against the craft-contrib licence list.

| Path | Source | Licence | Notes |
|---|---|---|---|
| `assets/brand/bayleaf-logo.png` | Bayleaf Engine project owner, own work | Project brand asset | The logo, on a transparent background; embedded as the window icon and menu bar mark, shown in the README |
| `examples/demo/materials/*.bmaterial`, `examples/demo/scripts/bob.bscript` | Bayleaf Engine project, own work | Project licence | Demo material graphs and visual script, made with the `graph.*` commands |
| `examples/showcase/models/windmill.gltf`, `windmill.bin` | Bayleaf Engine project, own work | Project licence | Tower, sails and their `Spin` animation, written by a script that builds boxes and quaternion keys |
| `examples/showcase/models/crate.obj`, `crate.mtl`, `crate.png` | Bayleaf Engine project, own work | Project licence | A cube written by hand; the plank texture drawn by a script (stripes and a sine grain) |
| `examples/showcase/post/warm.png` | Bayleaf Engine project, own work | Project licence | A 16-step colour lookup image computed by a script (red lifted, blue lowered) |
| `examples/showcase/ui/*.bui`, `post/showcase.bpost`, `scenes/main.bscene` | Bayleaf Engine project, own work | Project licence | Made by `examples/showcase/build.json` with `bayleaf-cli batch` |
| `docs/images/showcase-game.png`, `showcase-editor.png`, `interface-panel.png` | Bayleaf Engine project, own work | Project licence | The editor drawn offscreen by its UI test harness on the GPU, with the showcase open and playing, for the README |
