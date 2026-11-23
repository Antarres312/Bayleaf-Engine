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
convert into each other. A vector cannot feed a number input: use Split Vector or Vector Length.

A math node's Number or Vector output gives what its inputs make it, and the editor shows it so (its
colour and name): a vector when one of its number-or-vector inputs is a vector, linked or typed in;
else, in visual scripts, a whole number when every number input is whole (Sine and Square Root give
a number even then); else a number. A link is checked by that type, so a vector cannot go into a
number input through Add, and a new link that would turn an output further on into one is refused.
A file saved with such a link still opens; its compiler names the node to mend. `graph.get` gives
the same types as `outputTypes`.

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
  "version": 3,
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

A visual script may also have `variables`: each with a `name`, a `type` (`number`, `wholeNumber`, `vector`,
`trueFalse`, `text`, `object` or `color`) and a `start` value that fits the type (text of at most 256
characters; `null` for an object). A graph without them leaves the field out.
Limit: 256 variables. Version 1 files, which have no variables, still open.

A visual script may also have a `menu`: where Add Component lists it, folders then its name
(`Light/Flicker`, 1 to 4 parts); without it the script is under Scripts. Version 2 files kept it in a
Menu Path node; opening one moves the node's path into `menu`.

## Materials

A mesh uses a material when its `material` field names a `.bmaterial` file, relative to the project folder (or, for a scene outside any project, the scene's
folder). In the Inspector, pick it from the Mesh card's Material list, or drag a `.bmaterial` from the
Project panel onto that list. The **Object Color** node reads
the mesh's colour, so one material can be tinted per object. A material whose opacity is linked, or
set below 1, is drawn as transparent. Materials that use **Time** animate in the scene view.
The **Image** node reads a PNG or JPEG file (relative to the project folder) by the surface's texture
coordinates; **Texture Coordinates** gives them as a vector, to scale or move them. A material may
use up to 4 images.

Patterns are made from a position (the surface's place in the world unless one is linked) or texture
coordinates: **Checker**, **Noise**, **Fractal Noise** (noise in layers: Smooth, Ridged or Billowy, with
Detail, Roughness and Lacunarity), **Voronoi** (cells round scattered points: Distance, Edge and a
Cell Value, with Randomness), **Waves** (Bands or Rings, with Distortion), **Gradient** (Linear,
Quadratic, or Spherical and Radial round the middle of texture coordinates) and **Random** (white
noise, as a number and a colour). **Edge Glow** is bright where the surface turns away from the eye.

For a stylised look (cel shading, say), **Main Light** gives the main light at the point (the
direction towards it, its colour, and how much of it the shadows let through), **Other Lights** the
light the others give, added up, and **View Direction** and **Screen Position** where the point is
seen from. **Dot Product**, **Normalize**, **Step**, **Smooth Step**, **Posterize** (flat bands),
**Remap**, **Power**, **Square Root**, **Vector Length**, **Clamp 0 to 1**, **Lerp** (from A to B by a factor),
**Inverse Lerp** (where a value lies from A to B, 0 to 1), **Split Vector** and
**Make Vector** shape them, and
an **Image** read at a computed coordinate makes a ramp. A **Custom Lighting Output** in place of the
Material Output shows its colour as it is: the graph does the lighting. Either output has an outline
option (Outline Width in world units, up to 1, and Outline Color): the shape's back faces, grown along
their normals, draw a rim around it; smooth shapes outline cleanly, and hard corners (a cube's) can
show gaps. `examples/demo/materials/toon.bmaterial` puts them together.

While a material is open in the graph editor, the scene view shows its unsaved changes, and a
preview in the canvas corner shows it on a sphere (Hide puts it away; Show Preview in the toolbar
brings it back). A material
that does not load or compile is reported in the Inspector, and `scene.render` lists it in
`warnings`. Its meshes are drawn with their own colour, metallic and roughness.

## Visual scripts

Add a `.bscript` file to an object like any script (Inspector, Add Component, Scripts). It runs like
a `.wasm` script: same limits, same console, same hot reload when the file is saved during play.

- **On Init** runs once when the object is ready. When play starts (or a prefab is placed), every
  script loads first, then every script runs On Init, then every script runs On Start, so On Init
  and On Start see all the others whatever the hierarchy order.
- **On Start** and **On Update** begin chains of actions joined by flow links. On Update's
  **Step Seconds** output is the step length.
- **Branch** continues with True or False.
- **Properties** are values the Inspector shows per object. The script declares each in its panel
  with a name, a type (number, whole number, true/false, vector, colour, text or object) and a
  default (graph version 5; older scripts' Property nodes move their defaults there when opened).
  A **Property** node reads one (for an Object Property, the object of the scene chosen per object,
  from a list or by dragging it from the hierarchy; Found is false while none is chosen), and a
  **Set Property** node sets it on this object for the rest of play.
- **Negate** turns the sign of a number, a whole number or a vector. **Power** raises a number, a whole
  number (staying whole) or each part of a vector; **Square Root** takes the root of a number or of each
  part (0 below 0); **Vector Length** says how long a vector is, and **Normalize** gives its
  direction at length 1 (a vector of length 0 stays 0), so a move made of two keys is as fast
  diagonally as straight. **Lerp** goes from A (factor 0) to B (factor 1) on a straight line, for
  numbers or each part of vectors, and **Inverse Lerp** says where a value lies from A to B (0 when
  they are equal). Get and Set Component read and set another script's number,
  whole-number, true/false, vector and colour properties as pins of those types.
- **Set Variable** keeps a number under a name and **Variable** reads it; **Set Vector Variable**
  and **Vector Variable** do the same for vectors, **Set True/False Variable** and **True/False
  Variable** for true or false. **Set Text Variable** and **Text Variable** keep text (up to 1024 bytes;
  longer text is cut at a character), **Set Object Variable** and **Object Variable** an object (with
  Found: false before it is set and once the object is gone; unlike other object values it lasts from
  step to step), **Set Color Variable** and **Color Variable** a colour. Each script on each object has
  its own variables. A variable declared in the script's Variables panel starts at its **Starts at**
  value; one only named on nodes starts at 0 (0, 0, 0 for a vector, false for true/false, empty text,
  white for a colour). An object variable always starts as none.
- **Action** gives an input action's value by its kind (a trigger 1 in the step it goes down, held 1
  while down, value how far it is pressed) and whether it is down; **On Action Pressed** runs once
  when it goes down. Name it `Jump` for the active map or `Player/Jump`; only the active map's
  actions read. **Set Input Map** makes another map active, in the project's Input Map or this
  object's own. Prefer actions to keys, so players can rebind them.
- **On Key Pressed** runs once when its key goes down; **On Timer** runs every given number of
  seconds of play.
- **Walk**, **Jump** and **On Ground** steer an object with a Character: it walks at the given velocity
  until changed, climbs steps and gentle slopes, slides along walls, and jumps from the ground.
- **Add Impulse**, **Set Velocity** and **Velocity** move physics bodies. **On Trigger Entered** and
  **On Trigger Left** run when another collider comes into or leaves a trigger (this object's, or one
  this object moves through), and **On Collision Started** and **On Collision Ended** when colliders
  start and stop touching; each gives the Other Object. (They are the `triggerEnter`,
  `triggerExit`, `collision` and `collisionEnd` messages, which On Message also takes.)
- **Game Value** reads a number the game keeps under a name; **Set Game Value** and **Change Game
  Value** write it, and every script sees it. Scope picks whose: Game, Player (the object named in
  Player, else the first player; a name that finds no player reads nothing and writes nowhere), This
  Object (the script's own object, when it has Player Control), or Saved (kept in the project from one
  play to the next, such as a best score). The state is empty when play starts (saved values aside):
  Value is 0 and Is Set is false until something sets it, and Change Game Value counts a name not set
  yet as 0. **End Game** ends the game won or lost;
  only the first ending counts, and every script then gets the message `gameWon` or `gameLost`.
- **Play Particles**, **Stop Particles** and **Burst Particles** (Count) drive a Particle Emitter.
- **Cast Ray** sends a ray and gives whether it hit another object's shape or sprite, how far, and where.
- **Interface** nodes work on an object's Interface (`docs/interface.md`): **On Interface Pressed**
  and **On Interface Changed** run when the player presses a button (or any element with On Click)
  or changes a toggle, slider or text field named in Element; **On Message** hears an element's own
  On Click message; **Set Interface Text**, **Set Interface Value** and **Show Interface**
  change an element (Show Interface with no Element shows or hides it all); **Add Interface
  Element** builds the interface while playing, adding an element of a kind or a part from a `.bui`
  file, named by its Name input, into the element named in Into; **Remove Interface Element** takes
  one away. **Interface Value** and **Interface Text** read an element as it shows (Found says
  whether there is one).
- **Gamepad Stick**, **Gamepad Trigger** and **Gamepad Button Held** read the first connected gamepad
  (see `docs/script-abi.md` for the button names).
- **Send Message** sends a message name and a number to the scripts of the first object with the
  given name, or of every object when no name is given. **On Message** runs when a message with its
  name arrives; its Value output is the number. Messages arrive after the step. A message sent
  while handling one arrives a step later.
- **Object** pins (diamonds) carry an object of the scene. **This Object** is the script's own;
  **Find Object Named** finds one by name; **Find Object With** finds one that has a component
  (`Point Light`, `Character`), a tag, or runs a script (by its name), with Index 0 for the first in
  hierarchy order; **Count Objects With** counts them; **For Each Object With** runs Each once per
  such object, with the object and its index, then Then. Searches see the scene as the step began and
  ignore case, spaces, `_` and `-`. Found is false when nothing matches. Their Component, Tag or Script is
  picked from a list of components, the project's scripts and the scene's tags, or set by dragging
  a script from the Project panel onto it.
- **Get Component** shows the fields of the component named in Component (Point Light: Color,
  Intensity, Range, Shadows) as outputs, read from the linked object (else this object) as the step
  began; a missing component reads as zeros. **Try Get Component** adds Found. **Set Component**
  sets the fields that have a link, after the step; the others are left unchanged. Changing
  Component changes the pins and drops links to fields the node no longer has. The components are
  Transform, Mesh, Sprite, Camera, the lights, Environment, Rigid Body, Collider, Character, Player
  Control, Game Rules and Joint (its motor and limits), with their number, vector, colour and true/false fields. Physics reads
  Rigid Body, Collider and Character when play starts, so Set Component refuses them.
  A script is a component too: pick it from the Scripts in the list, or drag it from the Project
  panel onto Component, and its properties become number pins (the node keeps their names in its
  `fields`, so the graph compiles on its own). While the graph is open, the editor updates those
  names when the script's properties change (one undo step; links to dropped properties go away);
  a script that does not build leaves them as they were. Reads see the other script's values as the step
  began; a set reaches it when the step ends.
- Position Of, Set Position Of, Send Message, Walk, Jump, Add Impulse, Set Velocity and **Destroy
  Object** take an Object input. Unlinked, they use the first object named in Object Name, else
  their own default (this object, or for Send Message every object).
- **Text** pins (squares) carry text; an unlinked one is typed in the node. **Text**, **Join Text**,
  **Text Length**, **Part of Text** (Start and Count in characters), **Find in Text** (the index in
  characters, -1 when missing), **Replace Text**, **Upper Case**, **Lower Case** and **Trim Text**
  work on it; **Same Text**, **Text Contains**, **Starts With** and **Ends With** compare it;
  **To Text** (a number, a whole number or a vector, as (x, y, z)) and **Text to Number** (with Valid) convert; **Name Of** gives an object's name.
  Characters are counted as written, so ż or ę count once; Upper and Lower Case change a to z only.
  **Log** writes its message, then its linked Text, then its linked Value; **Find Object Named**
  takes a linked Name. Text made while a script runs lives until that call ends: keep numbers, not
  text, in variables.
- **Whole Number** pins (rings with a dot) carry 32-bit whole numbers: **Whole Number** types one,
  **To Whole Number** makes a number whole (Nearest, with halves away from zero; Down; Up; Toward
  Zero), and **Remainder** gives what is left after a whole division. Add, Subtract, Multiply and
  Divide on two whole numbers stay whole (they wrap past the 32-bit range; Divide keeps the whole
  part and gives 0 for a divisor of 0); a whole number links anywhere a number does. Text Length,
  Find in Text, Count Objects With and For Each Object With's Index are whole numbers, and Log and
  To Text print whole numbers with every digit. **Set Whole Number Variable** and **Whole
  Number Variable** keep one.
- **Load Scene** loads a scene file after the step: in place of the playing scene (every script
  starts again, physics starts over, the game's values stay), or with **Add to This Scene** its
  objects join the playing one and their scripts start; **Unload Scene** removes what was added from
  that file. Drag a `.bscene` (or, for Spawn Prefab, a `.bprefab`) from the Project panel onto the
  node's file field. Stop still brings back the scene as edited.
- **Play Sound** and **Stop Sound** play or stop the linked object's Audio Source (else this
  object's); **Play Sound At** plays a sound file once from a position, at a volume (drag a sound
  file onto its Sound File).
- Data nodes have no side effects. They are computed where an action needs them.
- **Functions** are actions a script runs from several places. A function has a name and typed
  inputs and outputs (the types of variables). Its **Function** node begins its actions and gives
  its inputs as values, which are read only on those actions; a **Return** node there ends it and
  gives back its outputs (a function that ends without one gives back zeros). **Call Function** runs
  it with its inputs and then goes on, with what it gave back as outputs. A function may call
  itself. The nodes' pins follow the function when its inputs or outputs change (graph version 4).
- **Events** are named happenings an object's scripts answer, triggered by other scripts, by the
  interface or by the script itself. An event has a name and typed values (the types of variables)
  and is declared in the script that answers it (graph version 7, `events`). Its **On Event** node
  runs when the event is triggered on the script's object and gives the values, which are read only
  on its own actions. **Trigger Event** triggers it, with its values, on the linked object, else the
  first object named in Object Name, else every object; the On Event nodes run after the step. Under
  the hood an event is a message with values, so an interface element's **On Click** naming an event
  triggers it (its values read as zero), and On Message hears it too. `graph.event.add`, `.set` and
  `.remove` change the declaration; the nodes' pins follow it.

`bayleaf_describe` is generated from the Property nodes. `cargo test -p bayleaf-script` builds a
graph with every node kind and runs it.

## Commands

Several graphs can be open at once; the editor shows each in a tab of its own. The commands below
work on the graph **in use**: the one opened or activated last (or whose tab was last worked in).
Undo goes back through the steps of every open graph and the scene in the order they were made, and
a graph's step makes that graph the one in use again. Closing a graph takes its steps with it.

| Command | Params | Undo |
|---|---|---|
| `graph.catalog` | `{domain}` | no |
| `graph.new` | `{domain}`: a new graph, then in use | no |
| `graph.open` | `{path}`: opens it, or brings it forward when it is open | no |
| `graph.save` | `{path?}` | no |
| `graph.close` | `{id?}`: the graph in use, or the open graph `id`; the one opened before it is then in use | no |
| `graph.list` | none: `{graphs: [{id, domain, path, dirty, inUse}]}` | no |
| `graph.activate` | `{id}` or `{path}`: that open graph is then in use | no |
| `graph.get` | none: the graph in use (`id`), its file, whether it changed, and its problems | no |
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
| `graph.variable.add` | `{name, type, start?}`: declares a visual script variable | yes |
| `graph.variable.set` | `{name, rename?, start?}`; a rename renames it on its nodes too | yes |
| `graph.variable.remove` | `{name}`; its nodes keep the name and start from zero | yes |
| `graph.property.add` | `{name, type, default?}`: declares a visual script property | yes |
| `graph.property.set` | `{name, rename?, default?}`; a rename renames it on its nodes too | yes |
| `graph.property.remove` | `{name}`; its nodes stay and read zero where no object sets it | yes |
| `graph.function.add` | `{name}`: declares a function with no inputs or outputs | yes |
| `graph.function.set` | `{name, rename?, inputs?, outputs?}`, inputs and outputs `[{name, type}]`; its nodes follow, and links to pins that went away go | yes |
| `graph.function.remove` | `{name}`; its nodes stay and say they name no function | yes |
| `graph.menu.set` | `{path}`: a visual script's menu path; empty puts it under Scripts | yes |

Graph edits share the undo history with scene edits. Opening or closing a graph drops that graph's
steps. Automation file access to graphs is confined like any other file (`docs/automation.md`).

## Using the editor

| Action | Mouse | Keyboard |
|---|---|---|
| Add a node | Right-click the canvas, or Add Node; click a category to open it | Add Node, type to search, Enter |
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
| Pan and zoom | Hold the right (or middle) button and drag, also over a node; the mouse wheel | Ctrl + Plus, Minus; Ctrl + 0 or Fit All shows every node |
| Select nodes in a box | Drag the left button over the background (Ctrl or Shift adds) | |
| Save the graph | Save in the toolbar | Ctrl+S while the graph is in use (a node, the canvas or the script panel has the keyboard, or the graph panel was clicked last); a graph never saved asks where |

A visual script has a panel left of its canvas. **Variables** lists the script's variables: rename
one (its nodes follow), set where it **Starts at**, place a node that reads it (**Get**) or sets it
(**Set**), remove it, or **Add Variable** of a type. **Properties** lists the script's properties: rename
one (its nodes follow), set its **Default**, place a node that reads it (**Get**) or sets it while
the game plays (**Set**), remove it, or **Add Property** of a type, and under **How the Inspector
shows it** give it a label, a hint, a group, read-only, a range or slider (numbers), choices (whole
numbers and text) and a property it shows only for (graph version 6, `properties[].inspector`;
`graph.property.set` takes them as `inspector` and refuses what a script's layout may not say); **Functions** lists the
functions with their inputs and outputs (**Add Function** declares one and places its Function and
Return nodes; **Add Input** and **Add Output** of a type; **Call** places a Call Function node),
**Events** lists the events with their values (**Add Event** declares one and places its On Event
node; **Add Value** of a type; **On Event** and **Trigger** place their nodes; **Trigger Another
Script's Event** places a Trigger Event for an event another script of the project declares), and
**Script** has the **Menu path** field,
which keeps what is typed and says why while it does not read right yet. On a variable node, Name is a list of the script's variables of that type (and names
other nodes use without declaring them); **New Variable…** declares a new one and picks it.

Every node, pin and field has a name for screen readers that includes its links ("Input Base Color
(Color) of Material Output 1, linked from Color 2"). Pins differ by shape as well as colour: flow
pins are arrows and object pins diamonds.
