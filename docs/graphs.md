# Graphs: materials and visual scripts

Materials and visual scripts are both node graphs. They use the same model (`crates/graph`), the same
file format, the same `graph.*` commands and the same editor panel (the **Graph** tab beside the
scene view). What differs is the set of node kinds and what the graph compiles to:

| Domain | File | Compiles to | Node kinds and compiler |
|---|---|---|---|
| `material` | `.bmaterial` | a WGSL `surface` function in the mesh shader | `crates/render/src/material.rs` |
| `script` | `.bscript` | a WebAssembly module with script ABI v1 | `crates/script/src/visual.rs` |

## Model

A graph is nodes joined by links. Each node kind has typed input and output pins, and may have
settings (values that are not pins, such as a key name). An input without a link uses the value
stored on the node, or its default.

Pin types: **Flow** (the order of actions in visual scripts), **Number**, **Vector**, **Color**,
**True/False**, and **Number or Vector** for math nodes, which take their type from what is
linked. A number can feed a vector or colour input (all three parts get it). Vectors and colours
convert into each other. A vector cannot feed a number input: use Split Vector.

The model keeps every graph valid:
- A link joins an output to a compatible input on another node.
- A data input has at most one link, and so does a flow output. A new link replaces the old one.
- There are no loops.
- Some kinds, such as Material Output, appear only once.

What can only be known when compiling (a math node that turns out to be a vector feeding a number
input) is reported as a problem on that node. The editor lists problems under the canvas, and
`graph.get` returns them.

## Files

Pretty JSON:

```json
{
  "format": "bayleaf.graph",
  "version": 1,
  "domain": "material",
  "nodes": [
    { "id": 1, "kind": "output", "position": [360, 40], "values": { "roughness": 0.3 } },
    { "id": 2, "kind": "color", "position": [40, 40], "values": { "color": [0.9, 0.2, 0.1] } }
  ],
  "links": [ { "from": { "node": 2, "pin": "color" }, "to": { "node": 1, "pin": "baseColor" } } ]
}
```

Limits: 2000 nodes, 8000 links, text values of 256 characters.

A graph may also have `comments`: boxes on the canvas with a note, drawn behind the nodes. They do
not change what the graph compiles to. Each has an `id`, `text` (at most 2000 characters), a
`position` (its top-left corner) and a `size` of at least 120 × 60. A graph without comments leaves
the field out. Limit: 500 comments.

## Materials

A mesh uses a material when its `material` field names a `.bmaterial` file, relative to the project folder (or, for a scene outside any project, the scene's
folder). In the Inspector, pick it from the Mesh card's Material list. The **Object Color** node reads
the mesh's colour, so one material can be tinted per object. A material whose opacity is linked, or
set below 1, is drawn as transparent. Materials that use **Time** animate in the scene view.
The **Image** node reads a PNG or JPEG file (relative to the project folder) by the surface's texture
coordinates; **Texture Coordinates** gives them as a vector, to scale or move them. A material may
use up to 4 images.

While a material is open in the graph editor, the scene view shows its unsaved changes, and a
preview in the canvas corner shows it on a sphere (Hide puts it away; Show Preview in the toolbar
brings it back). A material
that does not load or compile is reported in the Inspector, and `scene.render` lists it in
`warnings`. Its meshes are drawn with their own colour, metallic and roughness.

## Visual scripts

Add a `.bscript` file to an object like any script (Inspector, Add Component, Scripts). It runs like
a `.wasm` script: same limits, same console, same hot reload when the file is saved during play.

- **On Start** and **On Update** begin chains of actions joined by flow links. On Update's
  **Step Seconds** output is the step length.
- **Branch** continues with True or False.
- **Property** nodes declare numbers that the Inspector shows per object, with a default.
- **Set Variable** keeps a number under a name and **Variable** reads it; **Set Vector Variable**
  and **Vector Variable** do the same for vectors, **Set True/False Variable** and **True/False
  Variable** for true or false. Each script on each object has its own variables; numbers and vectors
  start at 0, true/false at false.
- **Action** gives an input action's value by its kind (a trigger 1 in the step it goes down, held 1
  while down, value how far it is pressed) and whether it is down; **On Action Pressed** runs once
  when it goes down. Name it `Jump` for the active map or `Player/Jump`; only the active map's
  actions read. **Set Input Map** makes another map active, in the project's Input Map or this
  object's own. Prefer actions to keys, so players can rebind them.
- **Menu Path** says where Add Component lists the script (`Light/Flicker`); without it the script
  is under Scripts.
- **On Key Pressed** runs once when its key goes down; **On Timer** runs every given number of
  seconds of play.
- **Walk**, **Jump** and **On Ground** steer an object with a Character: it walks at the given velocity
  until changed, climbs steps and gentle slopes, slides along walls, and jumps from the ground.
- **Add Impulse**, **Set Velocity** and **Velocity** move physics bodies. **On Message** with `collision`,
  `collisionEnd`, `triggerEnter` or `triggerExit` runs when the object touches another collider.
- **Game Value** reads a number the game keeps under a name; **Set Game Value** and **Change Game
  Value** write it, and every script sees it. Scope picks whose: Game, Player (the object named in
  Player, else the first player; a name that finds no player reads nothing and writes nowhere), This
  Object (the script's own object, when it has Player Control), or Saved (kept in the project from one
  play to the next, such as a best score). The state is empty when play starts (saved values aside):
  Value is 0 and Is Set is false until something sets it, and Change Game Value counts a name not set
  yet as 0. **End Game** ends the game won or lost;
  only the first ending counts, and every script then gets the message `gameWon` or `gameLost`.
- **Raycast** sends a ray and gives whether it hit another object's shape or sprite, how far, and where.
- **Gamepad Stick**, **Gamepad Trigger** and **Gamepad Button Held** read the first connected gamepad
  (see `docs/script-abi.md` for the button names).
- **Send Message** sends a message name and a number to the scripts of the first object with the
  given name, or of every object when no name is given. **On Message** runs when a message with its
  name arrives; its Value output is the number. Messages arrive after the step. A message sent
  while handling one arrives a step later.
- Data nodes have no side effects. They are computed where an action needs them.

`bayleaf_describe` is generated from the Property nodes. `cargo test -p bayleaf-script` builds a
graph with every node kind and runs it.

## Commands

| Command | Params | Undo |
|---|---|---|
| `graph.catalog` | `{domain}` | no |
| `graph.new` | `{domain}` | no |
| `graph.open` | `{path}` | no |
| `graph.save` | `{path?}` | no |
| `graph.close` | none | no |
| `graph.get` | none: the graph, its file, whether it changed, and its problems | no |
| `graph.node.add` | `{kind, position: [x, y]}`, returns `{node}` | yes |
| `graph.node.remove` | `{node}` | yes |
| `graph.node.move` | `{node, position}` | yes |
| `graph.node.set` | `{node, name, value}` (`null` restores the default) | yes |
| `graph.link.add` | `{from: {node, pin}, to: {node, pin}}` | yes |
| `graph.link.remove` | `{node, pin}`: every link at that pin | yes |
| `graph.node.copy` | `{nodes: [id]}`, returns `{text}`: the nodes and the links between them | no |
| `graph.node.paste` | `{text, offset?}`, returns `{nodes, skipped}`: copies with new ids; a node a graph may have only once is skipped | yes |
| `graph.node.duplicate` | `{nodes: [id]}`, returns `{nodes, skipped}` | yes |
| `graph.comment.add` | `{position, size?, text?}`, returns `{comment}` | yes |
| `graph.comment.set` | `{comment, text?, position?, size?}` | yes |
| `graph.comment.remove` | `{comment}`; the nodes in it stay | yes |

Graph edits share the undo history with scene edits. Opening or closing a graph drops that graph's
steps. Automation file access to graphs is confined like any other file (`docs/automation.md`).

## Using the editor

| Action | Mouse | Keyboard |
|---|---|---|
| Add a node | Right-click the canvas, or Add Node | Add Node, type to search, Enter |
| Link two pins | Drag from one pin to the other | Enter on one pin, then Enter on the other (Escape cancels) |
| Link to a new node | Drag from a pin into empty space | |
| Move a link | Drag it away from its input | |
| Remove links | Right-click a pin, Remove Links | |
| Move a node | Drag its title | Tab to its title, then the arrow keys (Shift: finer) |
| Delete a node | Right-click its title, Delete Node | Tab to its title, then Delete |
| Select several nodes | Ctrl + click their titles | |
| Copy, cut, paste, duplicate | Right-click a title, Copy or Duplicate | Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+D |
| Add a comment | Add Comment (around the selected nodes) | Add Comment, type the text, Enter |
| Move a comment and its nodes | Drag its title | Tab to its title, then the arrow keys |
| Resize a comment | Drag its bottom-right corner | Tab to its title, then Ctrl + arrow keys |
| Edit or delete a comment | Double-click its title; right-click it | Enter; Delete |
| Pan and zoom | Drag the background; the mouse wheel | Ctrl + Plus, Minus; Ctrl + 0 or Fit All shows every node |

Every node, pin and field has a name for screen readers that includes its links ("Input Base Color
(Color) of Material Output 1, linked from Color 2"). Pins differ by shape as well as colour: flow
pins are arrows.
