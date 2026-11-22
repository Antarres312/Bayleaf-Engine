//! L2: game interfaces. A layout file (`.bui`) holds nested elements: panels, rows and columns,
//! text, images, buttons, toggles, sliders, text fields, lists and progress bars. A style file
//! (`.bstyle`) holds rules that pick elements by kind, class or name and set their look. Styles
//! cascade in one plain order (the kind's defaults, kind rules, class rules, name rules, then the
//! element's own), with no selectors beyond that, and every length has its unit written out.
//! [`layout`] places the elements on a screen with flexible boxes only.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

mod layout;
mod style;

pub use layout::{Placed, Rect, TextMeasure, layout};
pub use style::{Align, Color, Direction, Justify, Length, PROPERTIES, Rule, STYLE_EXTENSION, STYLE_FORMAT, Style, StyleSheet, TextAlign};

pub const LAYOUT_FORMAT: &str = "bayleaf.interface";
pub const LAYOUT_EXTENSION: &str = "bui";
pub const VERSION: u32 = 1;
/// Most elements in one layout.
pub const MAX_ELEMENTS: usize = 4096;
/// Deepest nesting.
pub const MAX_DEPTH: usize = 32;
/// Longest text in an element, in bytes.
pub const MAX_TEXT: usize = 4096;
/// Longest element name, in bytes.
pub const MAX_NAME: usize = 48;

/// What an element is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A box holding others, laid out as a column unless its style says otherwise.
    Panel,
    /// Children side by side.
    Row,
    /// Children one under another.
    Column,
    Text,
    Image,
    Button,
    /// On or off (`value` 1 or 0), with its text beside it.
    Toggle,
    /// A number from `min` to `max`.
    Slider,
    /// Text the player types.
    TextField,
    /// A column of entries that scrolls when it is taller than its box.
    List,
    /// A bar filled by `value`, 0 to 1.
    Progress,
}

impl Kind {
    pub const ALL: [Kind; 11] =
        [Kind::Panel, Kind::Row, Kind::Column, Kind::Text, Kind::Image, Kind::Button, Kind::Toggle, Kind::Slider, Kind::TextField, Kind::List, Kind::Progress];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Panel => "panel",
            Kind::Row => "row",
            Kind::Column => "column",
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Button => "button",
            Kind::Toggle => "toggle",
            Kind::Slider => "slider",
            Kind::TextField => "textField",
            Kind::List => "list",
            Kind::Progress => "progress",
        }
    }

    /// Whether the kind holds other elements.
    pub fn holds_children(self) -> bool {
        matches!(self, Kind::Panel | Kind::Row | Kind::Column | Kind::List)
    }

    /// Whether the player can press, change or type into it (and so move focus to it).
    pub fn is_control(self) -> bool {
        matches!(self, Kind::Button | Kind::Toggle | Kind::Slider | Kind::TextField)
    }
}

/// One element of a layout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Element {
    pub kind: Kind,
    /// How scripts and name rules find it; unique in the layout when set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Class rules that apply to it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classes: Vec<String>,
    /// What text, a button, a toggle or a text field shows.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// An image file (PNG or JPEG), relative to the project folder.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    /// A toggle's state (0 or 1), a slider's number, a progress bar's fill (0 to 1).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub value: f64,
    /// A slider's range.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub min: f64,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub max: f64,
    /// Shows a game state value instead of `value` or `text`, with no script: `game:score`, or
    /// `player:health` for the first player.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bind: String,
    /// Its own style, applied last: property name to value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub style: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Element>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

fn one() -> f64 {
    1.0
}

fn is_one(v: &f64) -> bool {
    *v == 1.0
}

impl Element {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            name: String::new(),
            classes: Vec::new(),
            text: String::new(),
            image: String::new(),
            value: 0.0,
            min: 0.0,
            max: 1.0,
            bind: String::new(),
            style: BTreeMap::new(),
            children: Vec::new(),
        }
    }

    /// Every element in this one's tree, this one first, depth first.
    pub fn walk(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        let mut pending = vec![self];
        while let Some(element) = pending.pop() {
            out.push(element);
            pending.extend(element.children.iter().rev());
        }
        out
    }
}

/// A layout file: the element tree and the style files it uses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Layout {
    pub format: String,
    pub version: u32,
    /// Style files (`.bstyle`), relative to the project folder, applied in this order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub styles: Vec<String>,
    pub root: Element,
}

impl Layout {
    /// A new layout: a full-screen panel with a title and a button.
    pub fn starter() -> Self {
        let mut root = Element::new(Kind::Panel);
        root.name = "Screen".to_owned();
        root.style.insert("width".to_owned(), "100%".to_owned());
        root.style.insert("height".to_owned(), "100%".to_owned());
        root.style.insert("alignItems".to_owned(), "center".to_owned());
        root.style.insert("justify".to_owned(), "center".to_owned());
        root.style.insert("gap".to_owned(), "16px".to_owned());
        let mut title = Element::new(Kind::Text);
        title.name = "Title".to_owned();
        title.text = "My Game".to_owned();
        title.style.insert("fontSize".to_owned(), "40px".to_owned());
        let mut start = Element::new(Kind::Button);
        start.name = "Start".to_owned();
        start.text = "Start".to_owned();
        root.children = vec![title, start];
        Self { format: LAYOUT_FORMAT.to_owned(), version: VERSION, styles: Vec::new(), root }
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        let layout: Self = serde_json::from_str(text).map_err(|e| format!("not an interface layout: {e}"))?;
        layout.validate()?;
        Ok(layout)
    }

    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if meta.len() > 4 << 20 {
            return Err(format!("{} is larger than 4 MiB", path.display()));
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::from_text(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Checks the format, the tree's size and depth, names, kinds that hold children, values and
    /// every inline style.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != LAYOUT_FORMAT {
            return Err(format!("the format is {:?}, not {LAYOUT_FORMAT:?}", self.format));
        }
        if self.version > VERSION {
            return Err(format!("layout version {} is newer than this engine reads ({VERSION})", self.version));
        }
        for style in &self.styles {
            if !style.to_ascii_lowercase().ends_with(&format!(".{STYLE_EXTENSION}")) || style.len() > 512 {
                return Err(format!("{style:?} is not a .{STYLE_EXTENSION} file"));
            }
        }
        let mut count = 0;
        let mut names = std::collections::HashSet::new();
        let mut pending = vec![(&self.root, 1usize)];
        while let Some((element, depth)) = pending.pop() {
            count += 1;
            if count > MAX_ELEMENTS {
                return Err(format!("a layout holds at most {MAX_ELEMENTS} elements"));
            }
            if depth > MAX_DEPTH {
                return Err(format!("elements nest at most {MAX_DEPTH} deep"));
            }
            check_element(element)?;
            if !element.name.is_empty() && !names.insert(element.name.as_str()) {
                return Err(format!("two elements are named {:?}; a name is used once", element.name));
            }
            pending.extend(element.children.iter().map(|c| (c, depth + 1)));
        }
        Ok(())
    }

    /// The element named `name`.
    pub fn find(&self, name: &str) -> Option<&Element> {
        self.root.walk().into_iter().find(|e| e.name == name)
    }

    /// The element at `at`: child indices from the root (empty for the root itself).
    pub fn element_at(&self, at: &[usize]) -> Option<&Element> {
        at.iter().try_fold(&self.root, |element, i| element.children.get(*i))
    }

    pub fn element_at_mut(&mut self, at: &[usize]) -> Option<&mut Element> {
        at.iter().try_fold(&mut self.root, |element, i| element.children.get_mut(*i))
    }
}

fn check_element(element: &Element) -> Result<(), String> {
    let what = if element.name.is_empty() { element.kind.name().to_owned() } else { format!("{} {:?}", element.kind.name(), element.name) };
    if !element.children.is_empty() && !element.kind.holds_children() {
        return Err(format!("{what} holds other elements; only panels, rows, columns and lists do"));
    }
    // Short enough that `pressed:<name>` fits in a message name.
    if element.name.len() > MAX_NAME || element.name.chars().any(char::is_control) || element.name.contains(':') {
        return Err(format!("{what}: a name has at most {MAX_NAME} bytes, and no control characters or colons"));
    }
    if element.text.len() > MAX_TEXT {
        return Err(format!("{what}: text is at most {MAX_TEXT} bytes"));
    }
    if element.classes.len() > 16 || element.classes.iter().any(|c| c.is_empty() || c.len() > 64 || c.contains(char::is_whitespace)) {
        return Err(format!("{what}: at most 16 classes, each 1 to 64 characters without spaces"));
    }
    if !element.image.is_empty() {
        let lower = element.image.to_ascii_lowercase();
        if element.image.len() > 512 || !(lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
            return Err(format!("{what}: image must be the path of a .png or .jpg file"));
        }
    }
    if !(element.value.is_finite() && element.min.is_finite() && element.max.is_finite()) || element.min > element.max {
        return Err(format!("{what}: value, min and max are finite numbers, min at most max"));
    }
    if !element.bind.is_empty() && bind_target(&element.bind).is_none() {
        return Err(format!("{what}: bind is game:<name> or player:<name>, not {:?}", element.bind));
    }
    for (property, value) in &element.style {
        style::check_property(property, value).map_err(|e| format!("{what}: {e}"))?;
    }
    Ok(())
}

/// Where a bound value comes from: the game's state or the first player's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindScope {
    Game,
    Player,
}

/// A `bind` text read: its scope and the value's name.
pub fn bind_target(bind: &str) -> Option<(BindScope, &str)> {
    let (scope, name) = bind.split_once(':')?;
    let scope = match scope {
        "game" => BindScope::Game,
        "player" => BindScope::Player,
        _ => return None,
    };
    (!name.is_empty() && name.len() <= 64).then_some((scope, name))
}

#[cfg(test)]
mod tests;
