//! Style files (`.bstyle`) and how an element's look is worked out. A rule picks elements by one
//! thing only, their kind, a class or their name, and sets properties. The cascade is fixed: the
//! kind's defaults, then kind rules, class rules and name rules (each in file order, later files
//! after earlier ones), then the element's own style. Lengths always carry their unit: `px`
//! (points, scaled with the interface), `%` of the parent's inner size, or `auto`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Element, Kind};

pub const STYLE_FORMAT: &str = "bayleaf.style";
pub const STYLE_EXTENSION: &str = "bstyle";
/// Most rules in one style file.
pub const MAX_RULES: usize = 1024;

/// Every style property, with a value to start from when one is added.
pub const PROPERTIES: [(&str, &str); 25] = [
    ("direction", "row"),
    ("gap", "8px"),
    ("padding", "8px"),
    ("width", "100px"),
    ("height", "40px"),
    ("minWidth", "0px"),
    ("minHeight", "0px"),
    ("maxWidth", "400px"),
    ("maxHeight", "400px"),
    ("grow", "1"),
    ("shrink", "1"),
    ("alignItems", "center"),
    ("alignSelf", "center"),
    ("justify", "center"),
    ("background", "#202020c0"),
    ("color", "#ffffff"),
    ("borderColor", "#808080"),
    ("borderWidth", "1px"),
    ("radius", "6px"),
    ("fontSize", "18px"),
    ("textAlign", "center"),
    ("visible", "true"),
    ("opacity", "1"),
    ("accent", "#5fb3f9"),
    ("pointer", "pass"),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StyleSheet {
    pub format: String,
    pub version: u32,
    pub rules: Vec<Rule>,
}

/// Sets properties on every element of a kind, with a class, or of a name: exactly one of them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<Kind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub set: BTreeMap<String, String>,
}

impl StyleSheet {
    pub fn new() -> Self {
        Self { format: STYLE_FORMAT.to_owned(), version: crate::VERSION, rules: Vec::new() }
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        let sheet: Self = serde_json::from_str(text).map_err(|e| format!("not a style file: {e}"))?;
        sheet.validate()?;
        Ok(sheet)
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

    pub fn validate(&self) -> Result<(), String> {
        if self.format != STYLE_FORMAT {
            return Err(format!("the format is {:?}, not {STYLE_FORMAT:?}", self.format));
        }
        if self.version > crate::VERSION {
            return Err(format!("style version {} is newer than this engine reads ({})", self.version, crate::VERSION));
        }
        if self.rules.len() > MAX_RULES {
            return Err(format!("a style file holds at most {MAX_RULES} rules"));
        }
        for (i, rule) in self.rules.iter().enumerate() {
            let picks = usize::from(rule.kind.is_some()) + usize::from(rule.class.is_some()) + usize::from(rule.name.is_some());
            if picks != 1 {
                return Err(format!("rule {}: picks elements by exactly one of kind, class or name", i + 1));
            }
            for (property, value) in &rule.set {
                check_property(property, value).map_err(|e| format!("rule {}: {e}", i + 1))?;
            }
        }
        Ok(())
    }
}

impl Default for StyleSheet {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Auto,
    Px(f32),
    /// Of the parent's inner size along the same axis.
    Percent(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Row,
    Column,
}

/// Where children sit across the box's direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

/// Where children sit along the box's direction, when they leave room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Justify {
    Start,
    Center,
    End,
    SpaceBetween,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Start,
    Center,
    End,
}

/// sRGB colour with alpha, as written (`#rrggbb` or `#rrggbbaa`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const NONE: Color = Color([0, 0, 0, 0]);
}

/// An element's look, worked out.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub direction: Direction,
    pub gap: f32,
    /// Top, right, bottom, left.
    pub padding: [f32; 4],
    pub width: Length,
    pub height: Length,
    pub min_width: Length,
    pub min_height: Length,
    pub max_width: Length,
    pub max_height: Length,
    /// How much of the room left over it takes, against its siblings.
    pub grow: f32,
    /// How much it gives up when there is too little room: text gives room (it wraps); boxes,
    /// controls and images keep their size unless their style says otherwise.
    pub shrink: f32,
    pub align_items: Align,
    /// Its own place across its parent, instead of the parent's `alignItems`.
    pub align_self: Option<Align>,
    pub justify: Justify,
    pub background: Color,
    pub color: Color,
    pub border_color: Color,
    pub border_width: f32,
    pub radius: f32,
    pub font_size: f32,
    pub text_align: TextAlign,
    pub visible: bool,
    pub opacity: f32,
    /// A control's own colour: a toggle's mark, a slider's fill, a progress bar's fill, the focus ring.
    pub accent: Color,
    /// Whether it takes the pointer, so a click on it does not also reach the game; `None` for
    /// the default: controls, elements with On Click and boxes with a background take it.
    pub pointer: Option<bool>,
}

const WHITE: Color = Color([255, 255, 255, 255]);
const ACCENT: Color = Color([95, 179, 249, 255]);

impl Style {
    /// A kind's look before any rule.
    pub fn defaults(kind: Kind) -> Self {
        let mut style = Self {
            direction: Direction::Column,
            gap: 0.0,
            padding: [0.0; 4],
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::Auto,
            max_height: Length::Auto,
            grow: 0.0,
            shrink: if kind == Kind::Text { 1.0 } else { 0.0 },
            align_items: Align::Stretch,
            align_self: None,
            justify: Justify::Start,
            background: Color::NONE,
            color: WHITE,
            border_color: Color::NONE,
            border_width: 0.0,
            radius: 0.0,
            font_size: 18.0,
            text_align: TextAlign::Start,
            visible: true,
            opacity: 1.0,
            accent: ACCENT,
            pointer: None,
        };
        match kind {
            Kind::Row => style.direction = Direction::Row,
            Kind::List => style.gap = 4.0,
            Kind::Button => {
                style.padding = [8.0, 16.0, 8.0, 16.0];
                style.background = Color([45, 52, 64, 230]);
                style.radius = 6.0;
                style.text_align = TextAlign::Center;
            }
            Kind::Toggle => style.gap = 8.0,
            Kind::Slider => {
                style.width = Length::Px(200.0);
                style.height = Length::Px(24.0);
                style.background = Color([40, 40, 40, 200]);
                style.radius = 12.0;
            }
            Kind::TextField => {
                style.width = Length::Px(220.0);
                style.padding = [6.0, 8.0, 6.0, 8.0];
                style.background = Color([20, 20, 20, 220]);
                style.border_color = Color([120, 120, 120, 255]);
                style.border_width = 1.0;
                style.radius = 4.0;
            }
            Kind::Progress => {
                style.width = Length::Px(200.0);
                style.height = Length::Px(16.0);
                style.background = Color([40, 40, 40, 200]);
                style.radius = 8.0;
            }
            Kind::Panel | Kind::Column | Kind::Text | Kind::Image | Kind::Part => {}
        }
        style
    }

    /// The element's look: its kind's defaults, then the rules of `sheets` in the fixed order,
    /// then its own style. Values were checked when the files were read; one that does not read
    /// here is left out.
    pub fn of(element: &Element, sheets: &[StyleSheet]) -> Self {
        let mut style = Self::defaults(element.kind);
        let rules = || sheets.iter().flat_map(|s| s.rules.iter());
        for rule in rules().filter(|r| r.kind == Some(element.kind)) {
            style.apply_all(&rule.set);
        }
        for rule in rules().filter(|r| r.class.as_ref().is_some_and(|c| element.classes.contains(c))) {
            style.apply_all(&rule.set);
        }
        for rule in rules().filter(|r| r.name.as_ref().is_some_and(|n| *n == element.name && !n.is_empty())) {
            style.apply_all(&rule.set);
        }
        style.apply_all(&element.style);
        style
    }

    fn apply_all(&mut self, set: &BTreeMap<String, String>) {
        for (property, value) in set {
            let _ = self.apply(property, value);
        }
    }

    /// Sets one property from its written value.
    pub fn apply(&mut self, property: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        match property {
            "direction" => self.direction = keyword(value, &[("row", Direction::Row), ("column", Direction::Column)])?,
            "gap" => self.gap = px(value)?,
            "padding" => self.padding = padding(value)?,
            "width" => self.width = length(value)?,
            "height" => self.height = length(value)?,
            "minWidth" => self.min_width = length(value)?,
            "minHeight" => self.min_height = length(value)?,
            "maxWidth" => self.max_width = length(value)?,
            "maxHeight" => self.max_height = length(value)?,
            "grow" => self.grow = factor(value)?,
            "shrink" => self.shrink = factor(value)?,
            "alignItems" => self.align_items = align(value)?,
            "alignSelf" => self.align_self = Some(align(value)?),
            "justify" => {
                self.justify =
                    keyword(value, &[("start", Justify::Start), ("center", Justify::Center), ("end", Justify::End), ("spaceBetween", Justify::SpaceBetween)])?;
            }
            "background" => self.background = color(value)?,
            "color" => self.color = color(value)?,
            "borderColor" => self.border_color = color(value)?,
            "borderWidth" => self.border_width = px(value)?,
            "radius" => self.radius = px(value)?,
            "fontSize" => {
                let size = px(value)?;
                if !(4.0..=400.0).contains(&size) {
                    return Err("fontSize is between 4px and 400px".to_owned());
                }
                self.font_size = size;
            }
            "textAlign" => self.text_align = keyword(value, &[("start", TextAlign::Start), ("center", TextAlign::Center), ("end", TextAlign::End)])?,
            "visible" => self.visible = keyword(value, &[("true", true), ("false", false)])?,
            "opacity" => {
                let opacity = factor(value)?;
                if opacity > 1.0 {
                    return Err("opacity is from 0 to 1".to_owned());
                }
                self.opacity = opacity;
            }
            "accent" => self.accent = color(value)?,
            "pointer" => self.pointer = Some(keyword(value, &[("block", true), ("pass", false)])?),
            other => return Err(format!("{other:?} is not a style property; see the list in docs/interface.md")),
        }
        Ok(())
    }
}

/// Whether `value` is a valid value of `property`.
pub(crate) fn check_property(property: &str, value: &str) -> Result<(), String> {
    if value.len() > 128 {
        return Err(format!("{property}: a value is at most 128 characters"));
    }
    Style::defaults(Kind::Panel).apply(property, value).map_err(|e| if e.starts_with(property) || e.starts_with('"') { e } else { format!("{property}: {e}") })
}

fn keyword<T: Copy>(value: &str, words: &[(&str, T)]) -> Result<T, String> {
    words
        .iter()
        .find(|(w, _)| *w == value)
        .map(|(_, v)| *v)
        .ok_or_else(|| format!("{value:?} is not one of {}", words.iter().map(|(w, _)| *w).collect::<Vec<_>>().join(", ")))
}

fn number(text: &str) -> Option<f32> {
    text.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// A length in points: `12px`.
fn px(value: &str) -> Result<f32, String> {
    let n = value.strip_suffix("px").and_then(number).ok_or_else(|| format!("{value:?} is a length in points with its unit, like 12px"))?;
    if (0.0..=10_000.0).contains(&n) { Ok(n) } else { Err(format!("{value:?} is from 0px to 10000px")) }
}

/// `auto`, `12px` or `50%`.
fn length(value: &str) -> Result<Length, String> {
    if value == "auto" {
        return Ok(Length::Auto);
    }
    if let Some(n) = value.strip_suffix('%').and_then(number) {
        return if (0.0..=1000.0).contains(&n) { Ok(Length::Percent(n)) } else { Err(format!("{value:?} is from 0% to 1000%")) };
    }
    px(value).map(Length::Px).map_err(|_| format!("{value:?} is auto, a length like 12px, or a share like 50%"))
}

/// One, two (top and bottom, left and right) or four (top, right, bottom, left) lengths in points.
fn padding(value: &str) -> Result<[f32; 4], String> {
    let parts: Vec<f32> = value.split_whitespace().map(px).collect::<Result<_, _>>()?;
    match parts.as_slice() {
        [all] => Ok([*all; 4]),
        [vertical, horizontal] => Ok([*vertical, *horizontal, *vertical, *horizontal]),
        [top, right, bottom, left] => Ok([*top, *right, *bottom, *left]),
        _ => Err(format!("{value:?} is one, two or four lengths, like 8px 16px")),
    }
}

/// A plain number 0 or more (a share, not a length).
fn factor(value: &str) -> Result<f32, String> {
    number(value).filter(|n| (0.0..=1000.0).contains(n)).ok_or_else(|| format!("{value:?} is a number from 0 to 1000"))
}

fn align(value: &str) -> Result<Align, String> {
    keyword(value, &[("start", Align::Start), ("center", Align::Center), ("end", Align::End), ("stretch", Align::Stretch)])
}

/// `#rrggbb` or `#rrggbbaa`.
fn color(value: &str) -> Result<Color, String> {
    let hex = value.strip_prefix('#').filter(|h| (h.len() == 6 || h.len() == 8) && h.chars().all(|c| c.is_ascii_hexdigit()));
    let hex = hex.ok_or_else(|| format!("{value:?} is a colour like #ff8800 or #ff880080"))?;
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    Ok(Color([byte(0), byte(2), byte(4), if hex.len() == 8 { byte(6) } else { 255 }]))
}
