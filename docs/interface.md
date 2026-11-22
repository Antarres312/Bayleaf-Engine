# Game interfaces

A game builds its menus and its display from files of its own: a **layout** (`.bui`) of nested
elements and **style** files (`.bstyle`) of rules that set their look. An **Interface** component
puts a layout over the game. The model and the layout engine are `crates/interface`; drawing and
input are `crates/ui-egui/src/game_interface.rs`, used by the editor's Game view and the player.

## Layout files (`.bui`)

```json
{
  "format": "bayleaf.interface",
  "version": 1,
  "styles": ["ui/menu.bstyle"],
  "root": {
    "kind": "panel",
    "name": "Screen",
    "style": { "width": "100%", "height": "100%", "alignItems": "center", "justify": "center", "gap": "16px" },
    "children": [
      { "kind": "text", "name": "Title", "text": "My Game", "style": { "fontSize": "40px" } },
      { "kind": "button", "name": "Start", "text": "Start", "classes": ["big"] },
      { "kind": "text", "name": "Score", "bind": "game:score" }
    ]
  }
}
```

Each element has a `kind` and may have:

| Field | What it is |
| --- | --- |
| `name` | How scripts and name rules find it: once per layout, at most 48 bytes, no `:` |
| `classes` | Class rules that apply to it (up to 16) |
| `text` | What text, a button, a toggle or a text field shows |
| `image` | An image file (PNG or JPEG), relative to the project folder |
| `value` | A toggle's state (0 or 1), a slider's number, a progress bar's fill (0 to 1) |
| `min`, `max` | A slider's range (0 and 1 when left out) |
| `onClick` | The message the object's scripts get when the player clicks it, such as `openInventory`; any kind can have it (a panel, an image, a text), and it then answers the pointer and the keyboard as a button does |
| `bind` | Shows a game state value with no script: `game:<name>`, or `player:<name>` for the first player. Text and buttons show it as text; other kinds take it as their value |
| `style` | Its own style properties, applied last |
| `children` | The elements inside it; only panels, rows, columns and lists hold others |

Kinds: `panel` (a box, its children in a column unless its style says otherwise), `row`, `column`,
`text`, `image`, `button`, `toggle`, `slider`, `textField`, `list` (a column that cuts off entries
past its bottom and scrolls: under the mouse wheel while the game plays, or by its `value`, in points,
with a bar along its side), `progress`, and `part` (below).

## Parts

A layout can be used inside another as a **part**: an element `{"kind": "part", "name": "Character",
"layout": "ui/character.bui"}` shows that layout's root in its place, so a character menu made once
can go into the main display, twice if needed. The part takes the part element's name, and its own
classes and style come after the part layout's root's (the bigger layout sizes and places it); the
part's style files come along. Named elements inside it are called `<part>/<name>` (`Character/Health`),
so two uses never clash: scripts set `Character/Health` and hear `pressed:Character/Use`. Parts nest
up to 8 deep; one inside itself, a missing file, or a name grown past 56 bytes is an error, shown
over the game. Changing a part's file changes every layout that uses it. A part element's `texts`
(`{"Name": "Rogue"}`, layout version 2) gives named elements inside it other text where it is used,
by their names in the part (a name not in it is an error); the Interface panel shows a row for each.

In the Interface panel, **Save as Part** writes the picked element and what it holds to a layout
file of its own beside the layout (named after it) and puts a part using it in its place; **Open
Part** edits a picked part's layout. A row dragged in the tree goes before another, into it (its
middle, when it holds others) or after it; Up, Down, In and Out do the same from the keyboard.

A layout holds at most 4096 elements, nested at most 32 deep.

## Style files (`.bstyle`)

```json
{
  "format": "bayleaf.style",
  "version": 1,
  "rules": [
    { "kind": "button", "set": { "background": "#2d3440e6", "radius": "6px" } },
    { "class": "big", "set": { "fontSize": "28px", "padding": "12px 24px" } },
    { "name": "Start", "set": { "accent": "#ffb000" } }
  ]
}
```

A rule picks elements by exactly one thing: their `kind`, a `class` or their `name`. There are no
other selectors. The look of an element is worked out in one fixed order: its kind's defaults, then
kind rules, class rules and name rules (each in file order, the layout's style files in the order it
lists them), then its own `style`. Later wins.

## Properties

Every length has its unit written out: `px` (points, which the interface scales), `%` (of the
parent's inner size along the same axis) or `auto` (what the content needs). Shares (`grow`,
`shrink`, `opacity`) are plain numbers. Colours are `#rrggbb` or `#rrggbbaa`.

| Property | Values |
| --- | --- |
| `direction` | `row` or `column`: the way children are laid out |
| `gap` | a length in `px` between children |
| `padding` | one, two (top and bottom, then left and right) or four (top, right, bottom, left) lengths in `px` |
| `width`, `height`, `minWidth`, `minHeight`, `maxWidth`, `maxHeight` | `auto`, `px` or `%` |
| `grow` | how much of the room left over it takes, against its siblings (0 by default) |
| `shrink` | how much it gives up when there is too little room: 1 for text, which wraps; 0 for everything else, which keeps its size and runs past the box instead (a list's entries never shrink) |
| `alignItems` | `start`, `center`, `end` or `stretch`: where children sit across the direction |
| `alignSelf` | the same, for one element instead of its parent's `alignItems` |
| `justify` | `start`, `center`, `end` or `spaceBetween`: where children sit along the direction when they leave room |
| `background`, `color`, `borderColor`, `accent` | colours; `accent` is a control's own colour (a toggle's mark, a slider's and a progress bar's fill, the focus ring) |
| `borderWidth`, `radius` | lengths in `px` |
| `fontSize` | 4px to 400px |
| `textAlign` | `start`, `center` or `end` |
| `visible` | `true` or `false`; a hidden element takes no room |
| `opacity` | 0 to 1 |
| `pointer` | `block` or `pass`: whether it takes the pointer, so a click on it does not also reach the game. By default controls, elements with `onClick` and boxes with a background take it; text, images and empty boxes let it through |

Layout is flexible boxes only, one line each, so a layout reads the same way every time: a box lays
its children along its direction with its gap, gives the room left over to those that grow (or
takes it from those that shrink), and places them across by `alignItems`.

## Editing a layout

Double-click a layout in the Project panel (or Edit in Interface Panel in the Inspector) to open it
in the **Interface** panel. On the left is the element tree: Add puts a new element into the picked
one when it holds others, else after it; Remove, Up, Down, Out (out of the element holding it) and
In (into the element above it) rearrange; Delete on a focused row removes it. In the middle the
layout shows as a game would at the chosen screen size, and a click picks the element under the
pointer. On the right are the picked element's fields and its own style: a field is written when
you leave it, Add Property lists every style property with a value to start from, and a value that
does not read is refused with the reason in the status bar. Undo puts back the layout as it was
before the last change. Every change is a `ui.element.*` command, written at once.

## Editing a style file

Click a style file (`.bstyle`) in the Project panel to edit it in the Inspector: each rule in order,
what it picks elements by (a kind from the list, or a class or name typed) and its properties, with
Add Property, Up and Down (later rules win within their kind of rule), Remove Rule and Add Rule. A
value is written when its field is left; one that does not read is refused with the reason in the
status bar and stays to be fixed.

## The Interface component

`interface` has a `layout` (a `.bui` file), `visible`, and an `order`: interfaces with a higher order
draw over those with a lower one. The Game view shows interfaces while the game plays, and as a
preview before; the player draws them over the game.

## In the world

With `space: world` an Interface stands in the scene instead of over the screen: a flat panel at
its object, turned with it, `size` world units wide and high (`[2, 1]` by default), holding
`pointsPerUnit` interface points per world unit (200 by default; the picture is at most 2048 points
on a side). With `faceCamera` it always turns toward the camera, for a name tag or a health bar over
an enemy. The scene draws it like a sprite: hidden behind what stands in front of it, unlit, and
through the scene's post-processing, so its colours change with the tone mapping as the scene's do.
The Scene view shows it where it stands.

While the game plays, the pointer reaches the nearest panel it points at, through the scene, and
the panel's buttons, toggles, sliders and On Click elements answer as on the screen, with the same
messages to the scripts; a click on an element that takes the pointer is not also the game's.
Something in the scene in front of a panel keeps the pointer from it. A press on a panel gives it the
keyboard and the gamepad (typing goes into its focused text field, the D-pad moves between its
controls) until a press lands elsewhere. Commands and scripts treat a world interface as any other.
Not yet: a world panel's elements in the accessibility tree.

## While the game plays

The player moves the focus between buttons, toggles, sliders and text fields with Tab, presses with
the pointer, Space or Enter, and moves a focused slider with the arrow keys. With a gamepad, the
D-pad moves the focus to the nearest control that way (the first one when none has it) and the
bottom face button presses it; the game still reads the pad too. Each element is in the
accessibility tree with its role and its text. What the player does reaches the scripts of the
object that has the Interface:

- `pressed:<name>` when a button, or any named element with `onClick`, is pressed (value 0);
- the `onClick` message of the element clicked, as written (value 0), so several elements can
  share one handler: On Message in a visual script, `Script::message` in Rust;
- `changed:<name>` when a toggle, slider or text field changes, with the new value (a text field's
  value is its length in characters).

While the pointer is over an element that takes it, the game does not get its clicks: scripts read
no mouse button (`mouse_down`, actions bound to the mouse), and `pointer_over_interface` (Rust:
`pointer_over_interface()`; visual scripts: Pointer Position's Over the Interface) says so.

Scripts change elements with `ui_set_text`, `ui_set_value` and `ui_show`, and build the interface
while the game plays with `ui_add` (a new element of a kind, or a part from a layout file, into a
named element) and `ui_remove` (`docs/script-abi.md`); in the Rust SDK, `Object::set_interface_text`,
`set_interface_value`, `show_interface`, `add_interface_element` and `remove_interface_element`.
They read an element as it shows (what scripts and the player changed, bound values, what was
typed into a text field) with `ui_value` and `ui_text` (Rust: `Object::interface_value` and
`interface_text`; visual scripts: **Interface Value** and **Interface Text**).
Visual scripts have the same in the Interface category: **On Interface Pressed** and **On Interface
Changed** (Value is the new value), **Set Interface Text**, **Set Interface Value**, **Show
Interface**, **Add Interface Element** (What is a kind or a `.bui` file; the new element is named by
the Name input, so a list of slots can be built in a loop) and **Remove Interface Element**.
Stopping play puts every interface back as its files say.

## Commands

`ui.layout.get` and `ui.layout.set` (`{path, layout}`), `ui.style.get` and `ui.style.set`
(`{path, style}`) read and write the files, checking them first. Elements are addressed by their
place in the tree, child indices from the root (`[]` is the root, `[1, 0]` the first child of the
second): `ui.element.add` (`{path, into?, kind, index?}`), `ui.element.remove` (`{path, at}`),
`ui.element.move` (`{path, at, into, index}`) and `ui.element.set` (`{path, at, element}`, the fields
to change; `style` replaces the whole style) change one element, and write nothing when the result
would not read. `ui.element.saveAsPart` (`{path, at, to}`) writes an element to a layout file of its
own and puts a part in its place. While playing, `play.ui.state`
(`{id}`) gives an interface as it shows, `play.ui.press` (`{id, element}`) presses a button or flips a
toggle, and `play.ui.change` (`{id, element, value | text}`) moves a slider or types into a text
field, as the player would. `asset.create` makes a layout (`interface`) or a style file
(`interfaceStyle`).

## Not yet

Right-to-left text.
