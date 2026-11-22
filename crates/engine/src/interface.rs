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

/// One object's interface while the game plays: whether scripts showed or hid it, and its elements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InterfaceState {
    pub visible: Option<bool>,
    pub elements: HashMap<String, ElementState>,
}

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

/// A layout file and its style files, read.
pub struct Loaded {
    pub layout: Layout,
    pub styles: Vec<StyleSheet>,
}

/// Reads `layout` (relative to `base`) and the style files it names. A style file that does not
/// read is an error, so a mistake there shows instead of a plain look.
pub fn load(base: &Path, layout: &str) -> Result<Loaded, String> {
    let layout = Layout::load(&base.join(layout))?;
    let styles = layout.styles.iter().map(|s| StyleSheet::load(&base.join(s))).collect::<Result<Vec<_>, _>>()?;
    Ok(Loaded { layout, styles })
}

/// The element tree as it shows now: the file's, with what scripts and the player changed, and
/// bound values from the game's state.
pub fn shown(root: &Element, state: Option<&InterfaceState>, game: Option<&bayleaf_script::GameState>) -> Element {
    let mut root = root.clone();
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
pub(crate) fn apply(session: &mut Session, object: StableId, element: String, change: bayleaf_script::InterfaceChange) {
    let Some(play) = session.play.as_mut() else { return };
    let state = play.interfaces.entry(object).or_default();
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
    }
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
    let layout_path = session.allowed(&session.script_dir().join(&component.layout), Access::Read)?;
    let layout = Layout::load(&layout_path).map_err(CommandError::Failed)?;
    let mut styles = Vec::new();
    for style in &layout.styles {
        let path = session.allowed(&session.script_dir().join(style), Access::Read)?;
        styles.push(StyleSheet::load(&path).map_err(CommandError::Failed)?);
    }
    let state = session.interface_state(id);
    let visible = state.and_then(|s| s.visible).unwrap_or(component.visible);
    Ok((shown(&layout.root, state, session.game_state().as_ref()), visible, styles))
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
