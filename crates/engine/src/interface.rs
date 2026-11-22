//! Game interfaces: Interface components put a layout file (`.bui`) on screen, styled by its style
//! files (`.bstyle`). While the game plays, scripts change elements (`ui_set_text`, `ui_set_value`,
//! `ui_show`), elements bound to a game state value follow it, and what the player does to a
//! button, toggle, slider or text field reaches the object's scripts as a message
//! (`pressed:<name>`, `changed:<name>`). Play mode keeps those changes apart from the files, so
//! stopping puts every interface back as it was written.
//!
//! `ui.layout.*` and `ui.style.*` read and write the files; `play.ui.*` presses, changes and reads
//! elements of a playing game, as the player would.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bayleaf_core::StableId;
use bayleaf_interface::{BindScope, Element, Kind, Layout, StyleSheet, bind_target};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// What scripts and the player changed of one element while the game plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ElementState {
    pub text: Option<String>,
    pub value: Option<f64>,
    pub visible: Option<bool>,
}

/// One object's interface while the game plays: whether scripts showed or hid it, its elements,
/// and the elements scripts added and removed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InterfaceState {
    pub visible: Option<bool>,
    pub elements: HashMap<String, ElementState>,
    /// Added by scripts, in order: the name of the element each went into ("" for the root), and
    /// the element (a part already put in place).
    pub added: Vec<(String, Element)>,
    /// Named elements scripts removed.
    pub removed: Vec<String>,
}

/// Most elements scripts add to one interface while the game plays.
pub const MAX_ADDED: usize = 1024;

/// What the player did to an element.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A button was pressed (or a toggle flipped, which also changes its value).
    Pressed,
    /// A toggle, slider or progress bar has a new value.
    Changed(f64),
    /// A text field holds new text.
    Typed(String),
}

/// A layout file read, with its parts put in place and every style file it and its parts name.
pub struct Loaded {
    /// The layout as its file holds it (parts as parts), for editing.
    pub layout: Layout,
    /// What it shows: every part put in its place.
    pub root: Element,
    pub styles: Vec<StyleSheet>,
    /// The style and part files it took, relative to the project folder.
    pub files: Vec<String>,
}

/// Reads `layout` (relative to `base`), its parts and the style files they name, each path through
/// `allow` first (the session's confinement). A file that does not read is an error, so a mistake
/// shows instead of a plain look or a missing part.
pub fn load_with(base: &Path, layout: &str, allow: &dyn Fn(&Path) -> Result<PathBuf, String>) -> Result<Loaded, String> {
    let file = Layout::load(&allow(&base.join(layout))?)?;
    let mut read = |part: &str| allow(&base.join(part)).and_then(|p| Layout::load(&p));
    let expanded = bayleaf_interface::expand(&file, &mut read)?;
    let styles = expanded.styles.iter().map(|s| allow(&base.join(s)).and_then(|p| StyleSheet::load(&p))).collect::<Result<Vec<_>, _>>()?;
    let files = expanded.styles.iter().chain(&expanded.parts).cloned().collect();
    Ok(Loaded { layout: file, root: expanded.root, styles, files })
}

/// [`load_with`] with no confinement, for the editor's and the player's own views.
pub fn load(base: &Path, layout: &str) -> Result<Loaded, String> {
    load_with(base, layout, &|p| Ok(p.to_path_buf()))
}

/// The element tree as it shows now: the file's, with what scripts and the player changed, and
/// bound values from the game's state.
pub fn shown(root: &Element, state: Option<&InterfaceState>, game: Option<&bayleaf_script::GameState>) -> Element {
    let mut root = root.clone();
    if let Some(state) = state {
        for name in &state.removed {
            let mut pending = vec![&mut root];
            while let Some(element) = pending.pop() {
                element.children.retain(|c| c.name != *name);
                pending.extend(element.children.iter_mut());
            }
        }
        for (parent, added) in &state.added {
            if parent.is_empty() {
                if root.kind.holds_children() {
                    root.children.push(added.clone());
                }
                continue;
            }
            let mut pending = vec![&mut root];
            while let Some(element) = pending.pop() {
                if element.name == *parent && element.kind.holds_children() {
                    element.children.push(added.clone());
                    break;
                }
                pending.extend(element.children.iter_mut());
            }
        }
    }
    let mut pending = vec![&mut root];
    while let Some(element) = pending.pop() {
        if let Some((scope, name)) = bind_target(&element.bind)
            && let Some(game) = game
        {
            let value = match scope {
                BindScope::Game => game.game.get(name).copied(),
                BindScope::Player => game.players.values().next().and_then(|p| p.get(name)).copied(),
            };
            if let Some(value) = value {
                match element.kind {
                    Kind::Text | Kind::Button => element.text = if value.fract() == 0.0 { format!("{value:.0}") } else { format!("{value:.2}") },
                    _ => element.value = value,
                }
            }
        }
        if let Some(changed) = state.and_then(|s| s.elements.get(&element.name)).filter(|_| !element.name.is_empty()) {
            if let Some(text) = &changed.text {
                element.text = text.clone();
            }
            if let Some(value) = changed.value {
                element.value = value;
            }
            if let Some(visible) = changed.visible {
                element.style.insert("visible".to_owned(), visible.to_string());
            }
        }
        pending.extend(element.children.iter_mut());
    }
    root
}

impl Session {
    /// What scripts and the player changed of `object`'s interface while the game plays.
    pub fn interface_state(&self, object: StableId) -> Option<&InterfaceState> {
        self.play.as_ref().and_then(|p| p.interfaces.get(&object))
    }

    /// The player did something to `element` of `object`'s interface: the change is kept, and the
    /// object's scripts get `pressed:<name>` or `changed:<name>` with the new value.
    pub fn interface_input(&mut self, object: StableId, element: &str, input: Input) -> Result<(), String> {
        crate::play::playing(self)?;
        if element.is_empty() || element.len() > bayleaf_interface::MAX_NAME {
            return Err("name the element".to_owned());
        }
        let Some(play) = self.play.as_mut() else { return Err("play mode is not running".to_owned()) };
        let state = play.interfaces.entry(object).or_default().elements.entry(element.to_owned()).or_default();
        let (message, value) = match input {
            Input::Pressed => (format!("pressed:{element}"), 0.0),
            Input::Changed(value) => {
                state.value = Some(value);
                (format!("changed:{element}"), value as f32)
            }
            Input::Typed(text) => {
                let length = text.chars().count() as f32;
                state.text = Some(text);
                (format!("changed:{element}"), length)
            }
        };
        let input = std::sync::Arc::clone(&self.input);
        let Some(play) = self.play.as_mut() else { return Ok(()) };
        play.host.send(&mut self.scene, object, &message, value, play.time, &input);
        crate::play::after_message(self);
        Ok(())
    }
}

/// A script's change to an interface, applied after the step.
pub(crate) fn apply(session: &mut Session, by: StableId, object: StableId, element: String, change: bayleaf_script::InterfaceChange) {
    // An element made by a script: a kind by its name, or a part read now and put in place.
    let made = match &change {
        bayleaf_script::InterfaceChange::Add { what, .. } => match new_element(session, &element, what) {
            Ok(made) => Some(made),
            Err(problem) => {
                let who = session.scene.name(by).unwrap_or("Object").to_owned();
                session.log(crate::Level::Error, format!("[{who}] cannot add {element:?} to the interface: {problem}"));
                return;
            }
        },
        _ => None,
    };
    let Some(play) = session.play.as_mut() else { return };
    let state = play.interfaces.entry(object).or_default();
    if let bayleaf_script::InterfaceChange::Add { parent, .. } = &change {
        if state.added.len() < MAX_ADDED
            && let Some(made) = made
        {
            state.removed.retain(|n| *n != element);
            state.added.push((parent.clone(), made));
        }
        return;
    }
    if matches!(change, bayleaf_script::InterfaceChange::Remove) {
        state.added.retain(|(_, e)| e.name != element);
        if !state.removed.contains(&element) {
            state.removed.push(element);
        }
        return;
    }
    if element.is_empty() {
        if let bayleaf_script::InterfaceChange::Visible(visible) = change {
            state.visible = Some(visible);
        }
        return;
    }
    let slot = state.elements.entry(element).or_default();
    match change {
        bayleaf_script::InterfaceChange::Text(text) => slot.text = Some(text),
        bayleaf_script::InterfaceChange::Value(value) => slot.value = Some(value),
        bayleaf_script::InterfaceChange::Visible(visible) => slot.visible = Some(visible),
        bayleaf_script::InterfaceChange::Add { .. } | bayleaf_script::InterfaceChange::Remove => {}
    }
}

/// The element a script adds, named `name`: `what` is a kind (`button`, `text`, ...) or a layout
/// file (`.bui`), which comes in as a part put in place.
fn new_element(session: &Session, name: &str, what: &str) -> Result<Element, String> {
    if name.is_empty() || name.len() > bayleaf_interface::MAX_NAME || name.contains(':') {
        return Err(format!("name it with 1 to {} bytes and no colons", bayleaf_interface::MAX_NAME));
    }
    if what.to_ascii_lowercase().ends_with(&format!(".{}", bayleaf_interface::LAYOUT_EXTENSION)) {
        // Named before it is put in place, so its inner names become `<name>/<element>`, as in a file.
        let mut part = Element::new(Kind::Part);
        part.layout = what.to_owned();
        part.name = name.to_owned();
        let host = Layout { root: part, ..Layout::starter() };
        let base = session.script_dir();
        let mut read = |file: &str| session.allowed(&base.join(file), Access::Read).map_err(|e| e.to_string()).and_then(|p| Layout::load(&p));
        return Ok(bayleaf_interface::expand(&host, &mut read)?.root);
    }
    let kind = Kind::ALL.into_iter().find(|k| k.name() == what && *k != Kind::Part).ok_or_else(|| {
        format!("{what:?} is not a kind of element or a .bui file; kinds: {}", Kind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>().join(", "))
    })?;
    let mut element = Element::new(kind);
    name.clone_into(&mut element.name);
    Ok(element)
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let spec =
        |id, label, params, run| CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates: Mutates::No, enabled: always, run };
    vec![
        spec("ui.layout.get", "Read Interface", "{path}: an interface layout (.bui) as {layout: {format, version, styles, root}}", layout_get),
        spec(
            "ui.layout.set",
            "Write Interface",
            "{path, layout}: checks and writes an interface layout (.bui): elements {kind, name?, classes?, text?, image?, value?, min?, max?, bind?, style?, children?}",
            layout_set,
        ),
        spec(
            "ui.element.add",
            "Add Interface Element",
            "{path, into?, kind, index?}: adds an element of kind to a layout, inside the element at into (child indices from the root; the root when left out), at index or last. Returns {at}",
            element_add,
        ),
        spec(
            "ui.element.remove",
            "Remove Interface Element",
            "{path, at}: removes the element at at (child indices from the root) and what it holds",
            element_remove,
        ),
        spec(
            "ui.element.move",
            "Move Interface Element",
            "{path, at, into, index}: moves the element at at into the element at into, at index (counted without it). Returns {at}",
            element_move,
        ),
        spec(
            "ui.element.set",
            "Edit Interface Element",
            "{path, at, element: {kind?, name?, classes?, text?, image?, value?, min?, max?, bind?, style?}}: changes the given fields of the element at at; style replaces the element's whole style",
            element_set,
        ),
        spec(
            "ui.element.saveAsPart",
            "Save Interface Part",
            "{path, at, to}: writes the element at at, with what it holds, as a layout file of its own (to, a new .bui) and puts a part using it in its place; inside it, its names are then <part>/<name>",
            element_save_as_part,
        ),
        spec("ui.style.get", "Read Interface Style", "{path}: an interface style file (.bstyle) as {style: {format, version, rules}}", style_get),
        spec(
            "ui.style.set",
            "Write Interface Style",
            "{path, style}: checks and writes a style file (.bstyle): rules {kind | class | name, set: {property: value}}",
            style_set,
        ),
        spec(
            "play.ui.state",
            "Interface State",
            "{id}: while playing, the object's interface as it shows: {visible, elements: [{name, kind, text, value, visible}]} for its named elements",
            play_state,
        ),
        spec(
            "play.ui.press",
            "Press Interface Element",
            "{id, element}: while playing, presses a button (its scripts get pressed:<name>) or flips a toggle (changed:<name>), as the player would",
            play_press,
        ),
        spec(
            "play.ui.change",
            "Change Interface Element",
            "{id, element, value? | text?}: while playing, sets a slider's or toggle's value or a text field's text as the player would; its scripts get changed:<name>",
            play_change,
        ),
    ]
}

fn checked(session: &Session, path: &Path, extension: &str, access: Access) -> Result<PathBuf, CommandError> {
    if !path.to_string_lossy().to_ascii_lowercase().ends_with(&format!(".{extension}")) {
        return Err(CommandError::BadParams(format!("give a .{extension} file")));
    }
    session.allowed(&session.script_dir().join(path), access)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: PathBuf,
}

fn layout_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let path = checked(session, &p.path, bayleaf_interface::LAYOUT_EXTENSION, Access::Read)?;
    let layout = Layout::load(&path).map_err(CommandError::Failed)?;
    Ok(json!({ "layout": layout }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutSetParams {
    path: PathBuf,
    layout: Layout,
}

fn layout_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: LayoutSetParams = parse(params)?;
    p.layout.validate().map_err(CommandError::BadParams)?;
    let path = checked(session, &p.path, bayleaf_interface::LAYOUT_EXTENSION, Access::Write)?;
    std::fs::write(&path, p.layout.to_text()).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))?;
    Ok(json!({ "path": path }))
}

/// Reads a layout, changes it with `change`, checks the result and writes it; nothing is written
/// when the change or the check fails.
fn edit_layout<T>(session: &Session, path: &Path, change: impl FnOnce(&mut Layout) -> Result<T, String>) -> Result<T, CommandError> {
    let path = checked(session, path, bayleaf_interface::LAYOUT_EXTENSION, Access::Write)?;
    let mut layout = Layout::load(&path).map_err(CommandError::Failed)?;
    let out = change(&mut layout).map_err(CommandError::BadParams)?;
    layout.validate().map_err(CommandError::BadParams)?;
    std::fs::write(&path, layout.to_text()).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))?;
    Ok(out)
}

fn no_element(at: &[usize]) -> String {
    format!("there is no element at {at:?}")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddParams {
    path: PathBuf,
    /// The element to add into (child indices from the root; empty for the root).
    #[serde(default)]
    into: Vec<usize>,
    kind: Kind,
    /// Where among its children; the end when left out.
    #[serde(default)]
    index: Option<usize>,
}

fn element_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: AddParams = parse(params)?;
    let at = edit_layout(session, &p.path, |layout| {
        let parent = layout.element_at_mut(&p.into).ok_or_else(|| no_element(&p.into))?;
        if !parent.kind.holds_children() {
            return Err(format!("a {} holds no other elements; add into a panel, row, column or list", parent.kind.name()));
        }
        let index = p.index.unwrap_or(parent.children.len()).min(parent.children.len());
        let mut element = Element::new(p.kind);
        // Something to see and press, to change from.
        if matches!(p.kind, Kind::Text | Kind::Button | Kind::Toggle) {
            p.kind.name().clone_into(&mut element.text);
            element.text[..1].make_ascii_uppercase();
        }
        parent.children.insert(index, element);
        let mut at = p.into.clone();
        at.push(index);
        Ok(at)
    })?;
    Ok(json!({ "at": at }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AtParams {
    path: PathBuf,
    at: Vec<usize>,
}

fn element_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: AtParams = parse(params)?;
    edit_layout(session, &p.path, |layout| {
        let (index, parent) = p.at.split_last().ok_or("the root element stays; change it instead")?;
        let parent = layout.element_at_mut(parent).ok_or_else(|| no_element(&p.at))?;
        if *index >= parent.children.len() {
            return Err(no_element(&p.at));
        }
        parent.children.remove(*index);
        Ok(())
    })?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveParams {
    path: PathBuf,
    at: Vec<usize>,
    /// The element to move it into.
    into: Vec<usize>,
    /// Where among the new parent's children, counted after it is taken out.
    index: usize,
}

fn element_move(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: MoveParams = parse(params)?;
    let at = edit_layout(session, &p.path, |layout| {
        let (index, from) = p.at.split_last().ok_or("the root element cannot move")?;
        if p.into.starts_with(&p.at) {
            return Err("an element cannot move into itself".to_owned());
        }
        let target = layout.element_at(&p.into).ok_or_else(|| no_element(&p.into))?;
        if !target.kind.holds_children() {
            return Err(format!("a {} holds no other elements", target.kind.name()));
        }
        let parent = layout.element_at_mut(from).ok_or_else(|| no_element(&p.at))?;
        if *index >= parent.children.len() {
            return Err(no_element(&p.at));
        }
        let element = parent.children.remove(*index);
        // Taking it out shifts its later siblings, which may hold the target.
        let mut into = p.into.clone();
        if into.len() > from.len() && into.starts_with(from) && into[from.len()] > *index {
            into[from.len()] -= 1;
        }
        let target = layout.element_at_mut(&into).ok_or_else(|| no_element(&p.into))?;
        let index = p.index.min(target.children.len());
        target.children.insert(index, element);
        into.push(index);
        Ok(into)
    })?;
    Ok(json!({ "at": at }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    path: PathBuf,
    at: Vec<usize>,
    /// The fields to change: kind, name, classes, text, image, value, min, max, bind or style (the
    /// whole map). Children stay.
    element: serde_json::Map<String, Value>,
}

fn element_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SetParams = parse(params)?;
    if p.element.contains_key("children") {
        return Err(CommandError::BadParams("children change with ui.element.add, remove and move".to_owned()));
    }
    edit_layout(session, &p.path, |layout| {
        let element = layout.element_at_mut(&p.at).ok_or_else(|| no_element(&p.at))?;
        let mut value = serde_json::to_value(&*element).map_err(|e| e.to_string())?;
        for (key, field) in &p.element {
            value[key] = field.clone();
        }
        let mut changed: Element = serde_json::from_value(value).map_err(|e| format!("element: {e}"))?;
        changed.children = std::mem::take(&mut element.children);
        if !changed.children.is_empty() && !changed.kind.holds_children() {
            element.children = changed.children;
            return Err(format!("a {} holds no other elements; remove its children first", changed.kind.name()));
        }
        *element = changed;
        Ok(())
    })?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavePartParams {
    path: PathBuf,
    at: Vec<usize>,
    /// The new layout file (`.bui`), relative to the project folder; never over another file.
    to: String,
}

fn element_save_as_part(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SavePartParams = parse(params)?;
    if p.at.is_empty() {
        return Err(CommandError::BadParams("the root is the whole layout already; save a part of it".to_owned()));
    }
    let to = checked(session, Path::new(&p.to), bayleaf_interface::LAYOUT_EXTENSION, Access::Write)?;
    if to.exists() {
        return Err(CommandError::Failed(format!("{} already exists; pick another name", to.display())));
    }
    let source = checked(session, &p.path, bayleaf_interface::LAYOUT_EXTENSION, Access::Read)?;
    let host = Layout::load(&source).map_err(CommandError::Failed)?;
    let element = host.element_at(&p.at).cloned().ok_or_else(|| CommandError::BadParams(no_element(&p.at)))?;
    if element.kind == Kind::Part {
        return Err(CommandError::BadParams("it is a part already".to_owned()));
    }
    // The part keeps the style files the layout uses, so it looks the same on its own.
    let part = Layout { styles: host.styles.clone(), root: element.clone(), ..Layout::starter() };
    part.validate().map_err(CommandError::BadParams)?;
    // Names inside a part are its own: the bigger layout calls them `<part>/<name>`.
    edit_layout(session, &p.path, |layout| {
        let slot = layout.element_at_mut(&p.at).ok_or_else(|| no_element(&p.at))?;
        let mut used = Element::new(Kind::Part);
        used.name = element.name.clone();
        used.layout = p.to.replace('\\', "/");
        *slot = used;
        Ok(())
    })?;
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CommandError::Failed(format!("cannot make {}: {e}", parent.display())))?;
    }
    std::fs::write(&to, part.to_text()).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", to.display())))?;
    Ok(json!({ "path": to }))
}

fn style_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let path = checked(session, &p.path, bayleaf_interface::STYLE_EXTENSION, Access::Read)?;
    let style = StyleSheet::load(&path).map_err(CommandError::Failed)?;
    Ok(json!({ "style": style }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleSetParams {
    path: PathBuf,
    style: StyleSheet,
}

fn style_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: StyleSetParams = parse(params)?;
    p.style.validate().map_err(CommandError::BadParams)?;
    let path = checked(session, &p.path, bayleaf_interface::STYLE_EXTENSION, Access::Write)?;
    std::fs::write(&path, p.style.to_text()).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))?;
    Ok(json!({ "path": path }))
}

/// The playing object's interface, read, as it shows now: its elements, whether it shows at all,
/// and its style files.
fn playing_interface(session: &Session, id: StableId) -> Result<(Element, bool, Vec<StyleSheet>), CommandError> {
    crate::play::playing(session).map_err(CommandError::Failed)?;
    let component = session.scene.get::<bayleaf_scene::components::Interface>(id).ok_or_else(|| CommandError::BadParams(format!("{id} has no Interface")))?;
    if component.layout.is_empty() {
        return Err(CommandError::Failed("the Interface names no layout file".to_owned()));
    }
    let allow = |p: &Path| session.allowed(p, Access::Read).map_err(|e| e.to_string());
    let loaded = load_with(&session.script_dir(), &component.layout, &allow).map_err(CommandError::Failed)?;
    let state = session.interface_state(id);
    let visible = state.and_then(|s| s.visible).unwrap_or(component.visible);
    Ok((shown(&loaded.root, state, session.game_state().as_ref()), visible, loaded.styles))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdParams {
    id: StableId,
}

fn play_state(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let (root, visible, styles) = playing_interface(session, p.id)?;
    // Its own visibility, as its styles and what scripts set make it.
    let elements: Vec<Value> = root
        .walk()
        .into_iter()
        .filter(|e| !e.name.is_empty())
        .map(
            |e| json!({ "name": e.name, "kind": e.kind.name(), "text": e.text, "value": e.value, "visible": bayleaf_interface::Style::of(e, &styles).visible }),
        )
        .collect();
    Ok(json!({ "visible": visible, "elements": elements }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PressParams {
    id: StableId,
    element: String,
}

fn play_press(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PressParams = parse(params)?;
    let (root, _, _) = playing_interface(session, p.id)?;
    let element = root
        .walk()
        .into_iter()
        .find(|e| e.name == p.element)
        .cloned()
        .ok_or_else(|| CommandError::BadParams(format!("no element is named {:?}", p.element)))?;
    let input = match element.kind {
        Kind::Button => Input::Pressed,
        Kind::Toggle => Input::Changed(if element.value != 0.0 { 0.0 } else { 1.0 }),
        other => return Err(CommandError::BadParams(format!("{:?} is a {}; only buttons and toggles are pressed", p.element, other.name()))),
    };
    session.interface_input(p.id, &p.element, input).map_err(CommandError::Failed)?;
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeParams {
    id: StableId,
    element: String,
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    text: Option<String>,
}

fn play_change(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ChangeParams = parse(params)?;
    let (root, _, _) = playing_interface(session, p.id)?;
    let element = root
        .walk()
        .into_iter()
        .find(|e| e.name == p.element)
        .cloned()
        .ok_or_else(|| CommandError::BadParams(format!("no element is named {:?}", p.element)))?;
    let input = match (element.kind, p.value, p.text) {
        (Kind::Slider, Some(value), None) if value.is_finite() => Input::Changed(value.clamp(element.min, element.max)),
        (Kind::Toggle, Some(value), None) if value.is_finite() => Input::Changed(if value != 0.0 { 1.0 } else { 0.0 }),
        (Kind::TextField, None, Some(text)) if text.len() <= bayleaf_interface::MAX_TEXT => Input::Typed(text),
        (kind, _, _) => {
            return Err(CommandError::BadParams(format!(
                "{:?} is a {}: give a slider or toggle a finite value, or a text field its text (at most {} bytes)",
                p.element,
                kind.name(),
                bayleaf_interface::MAX_TEXT
            )));
        }
    };
    session.interface_input(p.id, &p.element, input).map_err(CommandError::Failed)?;
    Ok(json!({}))
}
