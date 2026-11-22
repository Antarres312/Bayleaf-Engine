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
| `bind` | Shows a game state value with no script: `game:<name>`, or `player:<name>` for the first player. Text and buttons show it as text; other kinds take it as their value |
| `style` | Its own style properties, applied last |
| `children` | The elements inside it; only panels, rows, columns and lists hold others |

Kinds: `panel` (a box, its children in a column unless its style says otherwise), `row`, `column`,
`text`, `image`, `button`, `toggle`, `slider`, `textField`, `list` (a column that cuts off entries
past its bottom) and `progress`.

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
| `shrink` | how much it gives up when there is too little room (1 by default; a list's entries never shrink) |
| `alignItems` | `start`, `center`, `end` or `stretch`: where children sit across the direction |
| `alignSelf` | the same, for one element instead of its parent's `alignItems` |
| `justify` | `start`, `center`, `end` or `spaceBetween`: where children sit along the direction when they leave room |
| `background`, `color`, `borderColor`, `accent` | colours; `accent` is a control's own colour (a toggle's mark, a slider's and a progress bar's fill, the focus ring) |
| `borderWidth`, `radius` | lengths in `px` |
| `fontSize` | 4px to 400px |
| `textAlign` | `start`, `center` or `end` |
| `visible` | `true` or `false`; a hidden element takes no room |
| `opacity` | 0 to 1 |

Layout is flexible boxes only, one line each, so a layout reads the same way every time: a box lays
its children along its direction with its gap, gives the room left over to those that grow (or
takes it from those that shrink), and places them across by `alignItems`.

## The Interface component

`interface` has a `layout` (a `.bui` file), `visible`, and an `order`: interfaces with a higher order
draw over those with a lower one. The Game view shows interfaces while the game plays, and as a
preview before; the player draws them over the game.

## While the game plays

The player moves the focus between buttons, toggles, sliders and text fields with Tab, presses with
the pointer, Space or Enter, and moves a focused slider with the arrow keys. Each element is in the
accessibility tree with its role and its text. What the player does reaches the scripts of the
object that has the Interface:

- `pressed:<name>` when a button is pressed (value 0);
- `changed:<name>` when a toggle, slider or text field changes, with the new value (a text field's
  value is its length in characters).

Scripts change elements with `ui_set_text`, `ui_set_value` and `ui_show` (`docs/script-abi.md`); in
the Rust SDK, `Object::set_interface_text`, `set_interface_value` and `show_interface`. Stopping
play puts every interface back as its files say.

## Commands

`ui.layout.get` and `ui.layout.set` (`{path, layout}`), `ui.style.get` and `ui.style.set`
(`{path, style}`) read and write the files, checking them first. While playing, `play.ui.state`
(`{id}`) gives an interface as it shows, `play.ui.press` (`{id, element}`) presses a button or flips a
toggle, and `play.ui.change` (`{id, element, value | text}`) moves a slider or types into a text
field, as the player would. `asset.create` makes a layout (`interface`) or a style file
(`interfaceStyle`).

## Not yet

Editing a layout visually in its own panel (the Inspector previews a layout at a chosen screen
size), interfaces on surfaces in the world, moving the focus with a gamepad, reading an element's
value or text from a script, visual script nodes for interfaces, scrolling lists, and right-to-left
text.
