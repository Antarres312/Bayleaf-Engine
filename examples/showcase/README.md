# Showcase

A Bayleaf Engine project that shows the engine's newer features in one scene. Open
`Showcase.bproject` in the editor and press Play.

- `models/windmill.gltf`: a tower and four sails; its animation `Spin` turns the sails once every
  4 seconds, played by the Windmill's Model Animation component
- `models/crate.obj`: a crate read with its MTL library (`crate.mtl`) and texture (`crate.png`)
- `ui/sign.bui`: the sign, an interface standing in the world; its button sends `wave`
- `ui/hud.bui` and `ui/card.bui`: the cards along the bottom, one part used five times, each with
  its own texts
- `post/showcase.bpost`: tone mapping, bloom, ambient occlusion, a vignette and the colour lookup
  image `post/warm.png`
- The Spark Orb's Particle Emitter starts sparks on its sphere's surface, with size and colour curves
- `materials/ground.bmaterial`: the floor, the logo (`assets/bayleaf-logo.png`) repeated five times
  across it and laid over a dark ground colour by its transparency

`build.json` is the `bayleaf-cli batch` recipe that made the interfaces, the post-processing and
the objects in a new project (`project.new`), with the models and images already in place (the
floor's material replaces the new project's chequered one):

```sh
cargo run -p bayleaf-cli -- batch --scene examples/showcase/scenes/main.bscene examples/showcase/build.json
```

Running it again on this project adds the objects a second time.
