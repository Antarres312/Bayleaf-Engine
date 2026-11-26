//! How a script's properties show in the Inspector, as the script declares it in `bayleaf_describe`
//! (`inspector`, see `docs/script-abi.md`): groups and foldouts, labels and hints, ranges and
//! sliders, lists of choices, read-only values, buttons that send the object a message, notes, and
//! items shown only while another property has a given value. It is data: no script code runs in
//! the editor, and the editor draws it with its own widgets.

use serde::Deserialize;

use crate::components::{DeclaredProperty, PropertyType, PropertyValue};

/// One item of a script's Inspector layout.
#[derive(Clone, Debug, PartialEq)]
pub enum InspectorItem {
    Property(PropertyLayout),
    /// A titled group of items; `closed` groups start folded.
    Group {
        title: String,
        closed: bool,
        items: Vec<InspectorItem>,
        show_if: Option<Condition>,
    },
    /// Sends the object the message `message` (value 0) while the game plays.
    Button {
        label: String,
        message: String,
        hint: String,
        show_if: Option<Condition>,
    },
    /// Text shown in the card; `warning` marks it as one.
    Note {
        text: String,
        warning: bool,
        show_if: Option<Condition>,
    },
}

/// How one declared property shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PropertyLayout {
    pub name: String,
    /// Shown instead of the name (made readable).
    pub label: Option<String>,
    /// Said on hover and to screen readers.
    pub hint: String,
    /// The lowest and highest value a number may be set to.
    pub range: Option<(f64, f64)>,
    /// Shown as a slider across `range`.
    pub slider: bool,
    /// Shown, not changed here.
    pub read_only: bool,
    /// A whole number chosen by its place in this list (0 first), or a text chosen from it.
    pub choices: Vec<String>,
    pub show_if: Option<Condition>,
}

/// Shown only while `property` has the value `equals`.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub property: String,
    pub equals: PropertyValue,
}

impl Condition {
    /// Whether `value` (the property's current value) meets the condition.
    pub fn holds(&self, value: &PropertyValue) -> bool {
        match (&self.equals, value) {
            (PropertyValue::Number(a), PropertyValue::Number(b)) => (a - b).abs() < 1e-9,
            (a, b) => a == b,
        }
    }
}

impl InspectorItem {
    pub fn show_if(&self) -> Option<&Condition> {
        match self {
            InspectorItem::Property(p) => p.show_if.as_ref(),
            InspectorItem::Group { show_if, .. } | InspectorItem::Button { show_if, .. } | InspectorItem::Note { show_if, .. } => show_if.as_ref(),
        }
    }
}

/// Most items in a layout, groups' items included.
pub const MAX_ITEMS: usize = 256;
/// Deepest nesting of groups.
pub const MAX_DEPTH: usize = 4;
/// Longest label, group title, button label or message name, in characters.
pub const MAX_LABEL: usize = 64;
/// Longest hint or note, in characters.
pub const MAX_TEXT: usize = 512;
/// Most choices of one property.
pub const MAX_CHOICES: usize = 64;

/// One item as written: exactly one of `property`, `group`, `button` and `note`, with the options
/// that kind takes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Raw {
    property: Option<String>,
    group: Option<String>,
    button: Option<String>,
    note: Option<String>,
    label: Option<String>,
    hint: Option<String>,
    range: Option<[f64; 2]>,
    slider: Option<bool>,
    read_only: Option<bool>,
    choices: Option<Vec<String>>,
    show_if: Option<(String, serde_json::Value)>,
    closed: Option<bool>,
    items: Option<Vec<serde_json::Value>>,
    message: Option<String>,
    warning: Option<bool>,
}

/// Reads and checks a layout against the script's declared properties: each item well formed,
/// each property it names declared (and placed once), options that fit the property's type, and
/// limits on size and nesting.
pub fn parse(value: &serde_json::Value, declared: &[DeclaredProperty]) -> Result<Vec<InspectorItem>, String> {
    let list = value.as_array().ok_or("inspector must be a list of items")?;
    let mut checker = Checker { declared, placed: Vec::new(), count: 0 };
    checker.items(list, 1)
}

struct Checker<'a> {
    declared: &'a [DeclaredProperty],
    placed: Vec<String>,
    count: usize,
}

fn short(what: &str, text: &str, limit: usize) -> Result<(), String> {
    let n = text.chars().count();
    if n == 0 || n > limit || text.chars().any(char::is_control) {
        return Err(format!("{what} {text:?} needs 1 to {limit} characters and no control characters"));
    }
    Ok(())
}

impl Checker<'_> {
    fn items(&mut self, list: &[serde_json::Value], depth: usize) -> Result<Vec<InspectorItem>, String> {
        if depth > MAX_DEPTH {
            return Err(format!("groups nest at most {MAX_DEPTH} deep"));
        }
        list.iter().map(|v| self.item(v, depth)).collect()
    }

    fn declared(&self, name: &str) -> Result<&DeclaredProperty, String> {
        self.declared.iter().find(|p| p.name == name).ok_or_else(|| format!("the layout names {name:?}, which the script does not declare"))
    }

    fn condition(&self, show_if: Option<(String, serde_json::Value)>) -> Result<Option<Condition>, String> {
        let Some((property, equals)) = show_if else { return Ok(None) };
        let declared = self.declared(&property)?;
        let equals = serde_json::from_value::<PropertyValue>(equals.clone())
            .ok()
            .filter(|v| v.fits(declared.ty) || (declared.ty == PropertyType::Object && matches!(v, PropertyValue::Text(_))))
            .ok_or_else(|| format!("showIf {property:?}: {equals} is not a {}", declared.ty.label()))?;
        Ok(Some(Condition { property, equals }))
    }

    fn item(&mut self, value: &serde_json::Value, depth: usize) -> Result<InspectorItem, String> {
        self.count += 1;
        if self.count > MAX_ITEMS {
            return Err(format!("a layout has at most {MAX_ITEMS} items"));
        }
        let raw: Raw = serde_json::from_value(value.clone()).map_err(|e| format!("item {value}: {e}"))?;
        let kinds = [raw.property.is_some(), raw.group.is_some(), raw.button.is_some(), raw.note.is_some()];
        if kinds.iter().filter(|k| **k).count() != 1 {
            return Err(format!("item {value}: each item is exactly one of property, group, button and note"));
        }
        let only = |allowed: &[&str]| -> Result<(), String> {
            let given = [
                ("label", raw.label.is_some()),
                ("hint", raw.hint.is_some()),
                ("range", raw.range.is_some()),
                ("slider", raw.slider.is_some()),
                ("readOnly", raw.read_only.is_some()),
                ("choices", raw.choices.is_some()),
                ("closed", raw.closed.is_some()),
                ("items", raw.items.is_some()),
                ("message", raw.message.is_some()),
                ("warning", raw.warning.is_some()),
            ];
            match given.iter().find(|(name, set)| *set && !allowed.contains(name)) {
                Some((name, _)) => Err(format!("item {value}: {name} does not go with this kind of item")),
                None => Ok(()),
            }
        };
        if let Some(hint) = &raw.hint {
            short("hint", hint, MAX_TEXT)?;
        }
        let show_if = self.condition(raw.show_if.clone())?;
        let hint = raw.hint.clone().unwrap_or_default();
        if let Some(name) = raw.property.clone() {
            only(&["label", "hint", "range", "slider", "readOnly", "choices"])?;
            return self.property(name, raw, hint, show_if).map(InspectorItem::Property);
        }
        if let Some(title) = raw.group {
            only(&["closed", "items"])?;
            short("group", &title, MAX_LABEL)?;
            let items = self.items(raw.items.as_deref().unwrap_or_default(), depth + 1)?;
            return Ok(InspectorItem::Group { title, closed: raw.closed.unwrap_or(false), items, show_if });
        }
        if let Some(label) = raw.button {
            only(&["hint", "message"])?;
            short("button", &label, MAX_LABEL)?;
            let message = raw.message.ok_or_else(|| format!("button {label:?} needs the message it sends"))?;
            short("message", &message, MAX_LABEL)?;
            return Ok(InspectorItem::Button { label, message, hint, show_if });
        }
        let text = raw.note.unwrap_or_default();
        only(&["warning"])?;
        short("note", &text, MAX_TEXT)?;
        Ok(InspectorItem::Note { text, warning: raw.warning.unwrap_or(false), show_if })
    }

    fn property(&mut self, name: String, raw: Raw, hint: String, show_if: Option<Condition>) -> Result<PropertyLayout, String> {
        let ty = self.declared(&name)?.ty;
        if self.placed.contains(&name) {
            return Err(format!("the layout places {name:?} twice"));
        }
        self.placed.push(name.clone());
        if let Some(label) = &raw.label {
            short("label", label, MAX_LABEL)?;
        }
        let range = match raw.range {
            Some([low, high]) if !matches!(ty, PropertyType::Number | PropertyType::WholeNumber) => {
                return Err(format!("{name:?}: a range is for numbers, not a {} ({low} to {high})", ty.label()));
            }
            Some([low, high]) if !(low.is_finite() && high.is_finite() && low < high) => {
                return Err(format!("{name:?}: the range needs two finite numbers, the lower first"));
            }
            other => other.map(|[low, high]| (low, high)),
        };
        let slider = raw.slider.unwrap_or(false);
        if slider && range.is_none() {
            return Err(format!("{name:?}: a slider needs a range"));
        }
        let choices = raw.choices.unwrap_or_default();
        if !choices.is_empty() {
            if !matches!(ty, PropertyType::WholeNumber | PropertyType::Text) {
                return Err(format!("{name:?}: choices are for a whole number (by place) or a text, not a {}", ty.label()));
            }
            if choices.len() > MAX_CHOICES {
                return Err(format!("{name:?}: at most {MAX_CHOICES} choices"));
            }
            for choice in &choices {
                short("choice", choice, MAX_LABEL)?;
            }
        }
        Ok(PropertyLayout { name, label: raw.label, hint, range, slider, read_only: raw.read_only.unwrap_or(false), choices, show_if })
    }
}
