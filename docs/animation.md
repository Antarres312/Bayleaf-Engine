# Skeletal animation

A glTF model with a skeleton (skinned parts, or parts on nodes that its animations move) is drawn by
its skin: each vertex follows up to four joints by its weights. Its animations are its **clips**.
This page says how clips play: on the GPU, faded and mixed, in layers, through animation states,
with clip events and root motion, and from scripts and commands. The code is in `crates/animation`
(clips, motions, players, animation states), `crates/assets/src/skeleton.rs` (poses and skins),
`crates/render` (GPU skinning) and `crates/engine/src/animation.rs` (players in the session and the
`animation.*` commands).

Behaviour is learned from public descriptions of skeletal animation (linear blend skinning,
cross-fades, one-dimensional blends, masked layers, state machines); no other engine's code, file
layout or wording is used.

## Skinning on the GPU

The renderer keeps one copy of a skinned model on the GPU, in its bound pose, with each vertex's four
joints and weights. Each frame it writes the matrices of every skinned object into one joint texture
(four texels a matrix, 256 matrices a row), and the vertex shader moves each vertex by its joints'
matrices; the shadow, outline and selection passes do the same. So many objects showing one model
share its vertices, and objects of one model and material are drawn in one call however differently
they are posed.

A model with a skeleton that nothing animates stands in its rest pose. The CPU path stays: a device
without the sixteen vertex inputs GPU skinning needs moves the vertices on the CPU and uploads them
per object, as `Renderer::set_cpu_skinning(true)` does for tests; both give the same picture. A
posed model's box (for picking, framing and shadows) is each joint's own box moved by its joint, put
together, so it is never smaller than the model.

## Clips

A clip is anything that poses named parts over time (`ClipSource` in `crates/animation/src/clip.rs`):
its length, where it puts the parts it moves at a moment, and the moments it marks. A model's own
animations are clips under their names. A clip made elsewhere (a timeline) becomes one by
implementing `ClipSource`, or by filling a `NamedClip` (keys of translation, rotation and scale for
parts by name, which are found in the skeleton by their node names); adding it to an object's
`Clips` under a name lets motions, animation states, scripts and commands play it like any other.

## Fades, mixes and layers

What plays is a **motion**:
- a **clip**, at a speed (0 to 10), looping or once (holding its last moment);
- a **mix** of motions by a number: each motion has a point, and the number picks the two points
  round it, each by how near it is (walk at 1 and run at 3, by speed; aim down at -45 and up at 45,
  by angle); before the first point the first motion alone, past the last the last alone. The motions
  of a mix play in step: each reaches its end when the mix does. The number is typed in or names a
  parameter;
- a **layer**: one motion over another on a joint and every joint under it only (a wave on the arm
  while the legs walk), by a weight from 0 to 1.

An object plays motions in layers: the base layer covers the whole skeleton, and each further layer
plays on a joint and the joints under it, over the base. Playing a motion on a layer fades it in over
the time chosen (0 to 60 seconds) from what played there; stopping the base layer fades to the rest
pose, and stopping another layer fades it out.

## Commands

| Command | Params | |
|---|---|---|
| `animation.get` | `{id}` | the object's clips (`{name, duration}`), joints, layers (`{joint, motion, time, fading}`), parameters and whether it plays, and `state` and `states` when it runs animation states |
| `animation.play` | `{id, clip? \| mix?: [{clip, at}], by?, layer?, fade?, loop?, speed?}` | plays a clip or a mix on the base layer, or on `layer` (a joint's name); `by` is a number or a parameter's name; `fade` defaults to 0.25 seconds |
| `animation.stop` | `{id, layer?, fade?}` | |
| `animation.parameter.set` | `{id, name, value}` | a number or true/false |
| `animation.trigger` | `{id, name}` | sets a trigger |
| `animation.state.set` | `{id, state, fade?}` | an object running animation states goes to that state now |
| `animation.joint.get` | `{id, joint}` | where a joint of the object's model is in the world now: `{position, rotation, scale}` |

In play mode every object with a Model Animation component gets a player when play starts (or when
it is spawned), which plays the component's clip; the commands then change what plays. Outside play
mode a command makes a player for the object (the Inspector's preview uses them); the scene file
never changes, and play mode starts and ends with none. Each command refuses a clip or joint the
model does not have, naming the ones it has.

## Animation states

A Model Animation component's **States** field names an animation states file (`.banim`, a graph
domain of its own: see `docs/graphs.md`). In play mode the object then runs it on its base layer
instead of playing the component's clip alone: it starts in the Start State, and each step takes the
first transition out of the state it is in whose condition holds, fading into the next state's
motion. Conditions read the object's parameters (numbers, true/false and triggers, which scripts and
`animation.parameter.set` / `animation.trigger` set; a trigger is used up by the transition it lets
through) and how long the state has played. A file that does not read or compile is reported in the
console, and the object then plays nothing; a clip it names that the model lacks is reported too.

## Clip events and root motion

A Model Animation's **Events** mark moments of its clips: `{clip, time, event}`, up to 256. When a
clip plays past one, the object's scripts get the script event of that name, with the object as its
one value (On Event in visual scripts; `message_name` in the ABI). A mix marks the moments of its
heaviest part; a layer those of its base and, at a weight of a half or more, of its top; a fading-out
motion marks none.

With **Root Motion** on, in play mode the root joint's movement along the ground (X and Z of the
model, with its import scale and axes) moves the object instead, through its own turn and scale,
and the joint is drawn where it stood at rest along the ground; its height stays in the pose. The
root joint is **Root Joint** by name, or else the first joint, parents first, that one of the model's
clips moves. A looping clip carries the object on across each start over rather than back. Turns
of the root joint stay in the pose.

## Following a joint

An object with a **Follow Joint** component (`followJoint`, `{joint}`) stands on that joint of its
parent's model (a sword in the hand), as the parent's animation moves it: its own Transform is then
measured from the joint, and the objects under it follow too. The parent needs a Mesh whose model
has a joint of that name; otherwise the object stays where its Transform puts it. It applies where
the scene is drawn (the views, `scene.render`, picking); the object's Transform itself does not
change, so scripts that need the joint read it (`animation.joint.get`, or the script functions
below).

## Scripts

Every script action is a command first. Rust scripts (`Object::play_clip`, `stop_clips`,
`set_animation_number`, `set_animation_flag`, `trigger_animation`, `animation_state` and `joint`)
and visual scripts (**Play Clip**, **Stop Clip**, **Set Animation Number**, **Set Animation Flag**,
**Trigger Animation**, **Animation State** and **Joint Position**, in the Animation category) reach
the host functions in `docs/script-abi.md`. Changes are made after the step, as `animation.play`,
`animation.stop`, `animation.parameter.set` and `animation.trigger` make them; what they cannot do
(a clip the model lacks) goes to the console with the script's object. Reads see the objects as the
last step posed them. A clip event arrives as a script event with the object as its value.

## Previewing in the editor

Outside play mode the Model Animation card of the Inspector previews a clip on the model: pick it
under **Preview**, **Play** or **Pause** it, drag **Time** to scrub, and **Stop** to end the preview;
the scene file never changes. The card does this with `animation.preview` (`{id, clip, time?,
playing?}`) and `animation.preview.stop` (`{id}`), which agents can use as well; both refuse in play
mode. The scene view's **Skeletons** switch draws the joints of every model with a skeleton (a line
from each joint to its parent, as the model is posed now); selected objects always show theirs.
