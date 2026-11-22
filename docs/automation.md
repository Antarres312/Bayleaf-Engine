# Automation

Every action in Bayleaf Engine is a registered command (`scene.object.create`, `scene.saveAs`,
`play.start`, …). The editor's panels, `bayleaf-cli` and AI agents all run the same commands, so an
agent can do everything a person can do in the editor. `bayleaf-cli commands` prints the full list
with each command's parameters.

There are two ways for an agent to work:

| Mode | What the agent drives | Start it with |
|---|---|---|
| Headless | a session of its own, no window | `bayleaf-cli mcp …` |
| Editor | a running editor, which you watch | `bayleaf --control <PORT> …`, then `bayleaf-cli mcp --bridge …` |

## Allowed folders

An agent may only read and write files inside the folders you name. Both kinds of folder are
required; without them the agent entry point refuses to start.

- `--automation-read-root <DIR>`: a folder the agent may read. Repeat for more.
- `--automation-write-root <DIR>`: a folder the agent may write and read. Repeat for more.

Relative paths in commands start at the first write root. Paths are resolved before the check, so
`..` and symbolic links cannot reach outside. A refused command fails with a message that names the
path and the allowed folders, and nothing is written. While playing, `audio.playing` lists the sounds playing, and `audio.play` and `audio.stop` play
or stop an object's Audio Source; agents' sessions play them silently, in time. `stream.areas` lists the
Streamed Areas with their state (away, waiting, reading, here or failed), and `stream.wait` waits until none is
being read, so an agent can step play and know what is there; `stream.split` (not while playing) cuts the
open scene into Streamed Areas and writes their scene files, which must be inside the write roots. While playing, `play.scene.load` (`{path, additive}`), `play.scene.unload` and `play.scene.list`
load scenes in place of the playing one or on top of it, as scripts can; the scene file and its
scripts must be inside the readable folders. Scripts that `play.start` would load must also
be inside a readable folder. `play.start` takes `{scripts?}`: `interpreted` (the default) or
`compiled` (machine code, in programs built with the `compiled-scripts` feature, as the player is);
a program without it interprets them and says so in the console. Its result names the `runtime` used.

The limits apply only to the agent's requests: in the editor, your own commands are not limited.

## Headless MCP server

```
bayleaf-cli mcp --automation-read-root examples/demo --automation-write-root out [--scene examples/demo/demo.bscene]
```

The server speaks the Model Context Protocol over stdio (JSON-RPC 2.0, one message per line). Each
command is one tool, named with `_` for `.` (`scene_object_create`). Commands that change nothing
are marked read-only. A failed command returns a tool result with `isError: true` and the reason.

### Resources

The server also offers resources, which a client can read and subscribe to:

| URI | Type | What it shows |
|---|---|---|
| `bayleaf://scene` | JSON | the open scene: every object and component (`scene.document`) |
| `bayleaf://scene/render` | PNG | the scene from its first camera, or framing every object, 640 × 360 |
| `bayleaf://graph` | JSON | the graph in use (of those open, each in a tab), its file and its problems |
| `bayleaf://play` | JSON | play mode: running or not, time, script errors |

After `resources/subscribe`, the server sends `notifications/resources/updated` when the resource
changes: after each request, and every half second while none arrives. That includes changes you
make in the editor in bridge mode. `scene.render` without a `path` returns the picture as base64 PNG
(`{png}`, at most 2048 pixels on a side); with a path it writes the file as before.

To add it to Claude Code, put this in the project's `.mcp.json` (paths are examples):

```json
{
  "mcpServers": {
    "bayleaf": {
      "command": "target/debug/bayleaf-cli",
      "args": ["mcp", "--automation-read-root", "examples", "--automation-write-root", "out"]
    }
  }
}
```

or run `claude mcp add bayleaf -- target/debug/bayleaf-cli mcp --automation-read-root examples --automation-write-root out`.
On Windows, write the program as `target/debug/bayleaf-cli.exe`. Build it first with
`cargo build -p bayleaf-cli`.

## Driving the editor

Start the editor with a control channel:

```
bayleaf examples/demo/demo.bscene --control 7878 --automation-read-root examples --automation-write-root out
```

It listens on `127.0.0.1` only and prints the address and the token file. The status bar shows
**Agent control** with the address while the channel is open, and each agent request shows its result
there. The token file is `bayleaf-control.token` in the system temp folder unless
`--control-token-file` names another; it is created with a new random token when missing.

Then point the MCP server at it:

```
bayleaf-cli mcp --bridge 127.0.0.1:7878 --control-token-file <token file> --automation-read-root examples --automation-write-root out
```

In bridge mode the editor enforces its own allowed folders; the server's roots are still required.
Agent requests run on the editor's UI thread between frames, exactly like a menu command, so they
appear in the undo history.

### Control channel protocol

Newline-delimited JSON over TCP. Every request has an `id`, a `method` and `params`; every reply has
the same `id`, `ok`, and either `result` or `error`.

```
→ {"id":1,"method":"auth","params":{"token":"<64 hex digits>"}}
← {"id":1,"ok":true,"result":{}}
→ {"id":2,"method":"engine.execute","params":{"command":"scene.object.create","params":{"name":"Box"}}}
← {"id":2,"ok":true,"result":{"id":"…"}}
```

- `auth` must come first; any other first request, or a wrong token, closes the connection.
- `engine.commands` returns the command list; `engine.execute` runs one command.
- Limits: 1 MiB per line, 16 connections, 5 minutes idle, 60 seconds per reply. After a timeout the
  command may still run; check the scene before sending it again.

On Windows the token file's inherited access rules are removed and only your account may read it
(with the system's `icacls`); on Linux and macOS it is readable by you only (mode 600). If that
fails, the editor does not open the control channel.

## What only the editor does

Everything that changes a scene, a graph, an asset, the project or play mode is a command, so
agents can do it too; they read the console with `console.read` and list the project's files with
`asset.list`. What stays in the editor is its own view: selection, the scene camera, the panel
layout (and maximised panels), recent projects, opening files with other programs, and building a
Rust script, which `script.build` refuses for agents because it runs the crate's own build code.

`scene.script.setProperty` takes a value of the property's type: a number, `true`/`false`, three numbers or text
(an object property's object name); `null` goes back to the script's default.

`project.export` (`{folder, platform?, mode?, player?}`) writes the open project as a game into a new
folder inside `folder`, named after the game: the player program under the game's name, and a `game`
folder of its scenes, assets and scripts (visual scripts compiled). The folder must be inside the
write roots; `player` (default: `bayleaf-player` beside the running program) must be readable.
`platform` is `windows`, `linux`, `mac` or `web` (default: this computer's) and `mode` how scripts
run: `interpreted`, `compiled` or `native` (default: the project's choice for the platform,
`settings.game.scriptModes`). `project.export.check` (same params, without `folder`) writes nothing
and returns `{ready, findings, scripts, files}`: each finding an `error`, `warning` or `note` with
its message and file, and how each script would run. The export refuses while the check finds an
error. For another desktop system, `player` must be a player built there (the check reads its first
bytes to tell which system a program is for), since this computer cannot build one; the web is not
there yet. A Linux game also gets `<game>.desktop`, its icon as `icon.png`, and `install.sh`, which
makes the program runnable (a copy made on Windows loses that) and adds the game to the applications
menu. `compiled` needs a desktop system and `native` the engine's source and cargo (and this
computer's system), and the check says when they are missing. In native mode
`project.export` builds the game's own player with its Rust scripts in (minutes the first time);
with `wait: false` it returns once the build can start, with `build: {folder, program}` and `native`
(the modules built in), and a program builds it with `native_export::build` in the background.

`scene.settings` returns the open scene's own settings (`{lighting}`) and `scene.settings.set`
(`{lighting?}`) changes them, as an undo step. A lighting asset (`.blight`) holds a scene's sky,
light around everything, fog and a sun for scenes without a Directional Light: `lighting.get`
(`{path}`) reads one and `lighting.set` (`{path, lighting}`) writes it, inside the write roots;
`asset.create` with kind `lighting` makes one.

`settings.graphics` in `project.settings.set` sets how every scene's shadows are drawn, in the
editor's views, `scene.render` and the exported game: `shadowResolution` (texels per side of each
shadow map: 512, 1024, 2048 or 4096), `shadowCascades` (how many maps a sun's shadow is split
into by distance from the camera in a large scene, 1 to 4), `shadowDistance` (how far a sun's
shadow reaches from the camera, in world units; 0 for the whole scene) and `shadowLights` (how
many lights cast shadows at once, 1 to 4, sun-like lights first). Project format version 7.

`play.message` (`{name, value?, id?}`) sends the scripts of object `id`, or of every object, a message
while the game plays, as an Inspector button of a script does. A script's description may lay out its
Inspector card (`inspector`, see `docs/script-abi.md`); `graph.property.set` takes a visual script
property's `inspector` options.

`prefab.open` (`{path}`) edits a prefab on its own: the scene (and its undo history) waits
underneath, and the open scene's commands then act on the prefab's objects, its root at the origin.
Prefabs open inside one another; `session.open_prefabs` lists them. `prefab.save` writes the
innermost one to its file, and `prefab.close` (`{discard?}`) goes back a level, refusing unsaved
changes unless `discard`; the copies underneath then follow the file, keeping their own changes.
While a prefab is open, play mode, saving the scene and opening another scene or project wait.

A post-processing asset (`.bpost`) says what a camera does to the drawn picture: `toneMapping`
(`enabled`, `exposure`), `bloom` (`enabled`, `strength`, `threshold`), `color` (`enabled`, `contrast`,
`saturation`, `lookup`, `lookupStrength`), `vignette` (`enabled`, `strength`, `smoothness`), `depthFog` (`enabled`, `color`,
`start`, `end`), `occlusion` (`enabled`, `strength`, `radius`) and `antialiasing`. `post.get`
(`{path}`) reads one and `post.set` (`{path, post}`) writes it, inside the write roots; `asset.create`
with kind `postProcessing` makes one. `scene.settings.set` takes `postProcessing` (the scene's asset),
a Camera component's `postProcessing` field names its own (used by the Game view, the player and
`scene.render` through that camera), and `settings.graphics.effects` in the project settings turns
effects off for every scene (project format 8; scene format 18).

`color.lookup` names a colour lookup image (post-processing version 2): N squares of N by N pixels
side by side (N² wide, N high, N from 2 to 64), one square per step of blue, red across each square
and green down it, all as sRGB. It maps each colour, after contrast and saturation, to the colour
found there, blended by `lookupStrength` (0 to 1). `post.set` refuses an image of another size and
warns of one not there yet. `post.lookup.neutral` (`{path, steps?}`, a `.png`, steps 16 unless given)
writes one that changes no colour, to recolour in an image editor; it returns `{path, relative}` and
does not write over a file.

A Particle Emitter component (`particleEmitter`, scene format 19) starts particles while the game
plays: its cycle (`playing`, `looping`, `duration`, `rate`, `burst`, `maxParticles`), where they start
(`shape`: `point`, `sphere`, `cone` or `box`, with `radius`, `angle` and `boxSize`), how they live
(`lifetime`, `speed`, `gravity`, `drag`, `spin`, and size and colour at the start, middle and end of
their life; `sizeKeys` and `colorKeys`, points `{at, size}` and `{at, color}`, replace start, middle
and end when there are any; `shape: surface` starts them on the object's own mesh) and how they draw (`render`: `sprite`, `stretched` or `mesh` with `meshShape`; `image`,
`flipbookColumns`/`flipbookRows`, `material`, `blend`: `alpha` or `additive`; `space`: `world` or
`local`). `play.status` counts the particles alive; scripts start, stop and burst emitters.

The profiler records while asked: `profile.start` and `profile.stop`, `profile.clear`, and
`profile.frames` (`{last?}`) returning each play step's milliseconds by part (`scripts` with
`byScript`, slowest first, `physics`, `streaming`, `sound`, `particles`), what the Game view or the
player drew (`extract`, `draw`, `drawCalls`, `triangles`, `meshBytes`, `imageBytes`), and the
average. It keeps the last 3000 frames. The player's `--profile <file>` records and writes them
when the game closes. `gpu` is the GPU's own time for a frame, measured with timestamp queries a frame
or two behind; `gpuMeasured` is false where the graphics device gives none (some drivers offer
them and write only zeros), and both are null before a view has drawn. `HeadlessGpu` and the
windows take `WGPU_BACKEND` and `WGPU_POWER_PREF` to choose another adapter.

Files come into a project with `asset.import` (`{files, folder?}`): models (`.glb`, or `.gltf` or `.obj` with
the files it names, which then get a folder of their own), images (`.png`, `.jpg`, `.exr`) and sounds
(WAV, Ogg Vorbis, FLAC, MP3 and others). Each is
read first, so a file that cannot be read says why and nothing is copied; a name already taken gets
a number, and a file already in the project stays where it is. The files must be inside the read
roots. Models and images get import settings beside them (`robot.glb.bimport`), read with
`asset.importSettings.get` and written with `asset.importSettings.set` (`{path, settings: {model?:
{scale, upAxis: "y" | "z"}, image?: {mipmaps}}}`); scenes using the file read it again. Then
`scene.file.place` (`{path, parent?, index?, position?}`) puts a model or a sound in the scene as an
object, and `scene.image.apply` (`{id, image}`) shows an image on a sprite, or on a mesh through a
new material made beside the image. Dropping files on the editor runs the same commands.

Game interfaces (`docs/interface.md`) are layout files (`.bui`) and style files (`.bstyle`):
`ui.layout.get` and `ui.layout.set` (`{path, layout}`), `ui.style.get` and `ui.style.set` (`{path,
style}`) read and write them, checked first; `ui.element.add`, `ui.element.remove`,
`ui.element.move` and `ui.element.set` change one element, named by its place in the tree (child
indices from the root), and `ui.element.saveAsPart` (`{path, at, to}`) makes an element a part of
its own, used in its place; `asset.create` makes them (kinds `interface` and `interfaceStyle`). While the game plays, `play.ui.state` (`{id}`) gives an object's interface as it
shows (`{visible, elements: [{name, kind, text, value, visible}]}`), `play.ui.press` (`{id,
element}`) presses a button or flips a toggle, and `play.ui.change` (`{id, element, value | text}`)
moves a slider or types into a text field, as the player would; the object's scripts hear it.
