//! How a script's properties show in the Inspector ([`crate::Script::INSPECTOR`]): groups and
//! foldouts, labels and hints, ranges and sliders, lists of choices, read-only values, buttons that
//! send the object a message while the game plays, notes, and items shown only while another
//! property has a given value. It is data the editor reads; no script code runs in the editor.
//!
//! ```ignore
//! const INSPECTOR: &'static [Layout] = &[
//!     Layout::group("Movement", &[
//!         Layout::property("speed").label("Top speed").slider(0.0, 20.0).hint("Units a second"),
//!         Layout::property("mode").choices(&["Walk", "Run"]),
//!         Layout::property("boost").when("mode", Is::Whole(1)),
//!     ]),
//!     Layout::button("Jump now", "jump"),
//!     Layout::warning("Keep speed under 10 on small maps."),
//! ];
//! ```
//! Properties the layout does not place follow it, one row each.

/// A value a property is compared with, for [`Layout::when`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Is {
    Number(f32),
    Whole(i32),
    Flag(bool),
    Text(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Property(&'static str),
    Group,
    Button,
    Note,
}

/// One item of a script's Inspector layout. Make it with [`Layout::property`], [`Layout::group`],
/// [`Layout::foldout`], [`Layout::button`], [`Layout::note`] or [`Layout::warning`], then add options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    kind: Kind,
    /// A property's label, a group's title, a button's label or a note's text.
    text: &'static str,
    hint: &'static str,
    range: Option<(f32, f32)>,
    slider: bool,
    read_only: bool,
    choices: &'static [&'static str],
    closed: bool,
    items: &'static [Layout],
    message: &'static str,
    warning: bool,
    when: Option<(&'static str, Is)>,
}

impl Layout {
    const fn new(kind: Kind, text: &'static str) -> Self {
        Self {
            kind,
            text,
            hint: "",
            range: None,
            slider: false,
            read_only: false,
            choices: &[],
            closed: false,
            items: &[],
            message: "",
            warning: false,
            when: None,
        }
    }

    /// The declared property `name`, placed here.
    pub const fn property(name: &'static str) -> Self {
        Self::new(Kind::Property(name), "")
    }

    /// A titled group of items, open at first.
    pub const fn group(title: &'static str, items: &'static [Layout]) -> Self {
        let mut layout = Self::new(Kind::Group, title);
        layout.items = items;
        layout
    }

    /// A titled group of items that starts folded.
    pub const fn foldout(title: &'static str, items: &'static [Layout]) -> Self {
        let mut layout = Self::group(title, items);
        layout.closed = true;
        layout
    }

    /// A button that sends this object's scripts the message `message` (value 0) while the game
    /// plays: handle it in [`crate::Script::message`].
    pub const fn button(label: &'static str, message: &'static str) -> Self {
        let mut layout = Self::new(Kind::Button, label);
        layout.message = message;
        layout
    }

    /// A line of text in the card.
    pub const fn note(text: &'static str) -> Self {
        Self::new(Kind::Note, text)
    }

    /// A line of text marked as a warning.
    pub const fn warning(text: &'static str) -> Self {
        let mut layout = Self::note(text);
        layout.warning = true;
        layout
    }

    /// A property's label instead of its name.
    pub const fn label(mut self, label: &'static str) -> Self {
        self.text = label;
        self
    }

    /// Said on hover and to screen readers (a property or a button).
    pub const fn hint(mut self, hint: &'static str) -> Self {
        self.hint = hint;
        self
    }

    /// The lowest and highest value a number property may be set to.
    pub const fn range(mut self, low: f32, high: f32) -> Self {
        self.range = Some((low, high));
        self
    }

    /// A number property shown as a slider from `low` to `high`.
    pub const fn slider(mut self, low: f32, high: f32) -> Self {
        self.range = Some((low, high));
        self.slider = true;
        self
    }

    /// Shown, not changed in the Inspector.
    pub const fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// A whole number chosen by its place in `choices` (0 first), or a text chosen from them.
    pub const fn choices(mut self, choices: &'static [&'static str]) -> Self {
        self.choices = choices;
        self
    }

    /// Shown only while the property `property` has the value `value`.
    pub const fn when(mut self, property: &'static str, value: Is) -> Self {
        self.when = Some((property, value));
        self
    }

    /// The item as the JSON `bayleaf_describe` lists it.
    pub(crate) fn json(&self) -> String {
        let text = |t: &str| format!("\"{}\"", crate::escape(t));
        let number = |v: f32| if v.is_finite() { format!("{v:?}") } else { "0.0".to_owned() };
        let mut fields = Vec::new();
        match self.kind {
            Kind::Property(name) => {
                fields.push(format!("\"property\": {}", text(name)));
                if !self.text.is_empty() {
                    fields.push(format!("\"label\": {}", text(self.text)));
                }
                if let Some((low, high)) = self.range {
                    fields.push(format!("\"range\": [{}, {}]", number(low), number(high)));
                }
                if self.slider {
                    fields.push("\"slider\": true".to_owned());
                }
                if self.read_only {
                    fields.push("\"readOnly\": true".to_owned());
                }
                if !self.choices.is_empty() {
                    fields.push(format!("\"choices\": [{}]", self.choices.iter().map(|c| text(c)).collect::<Vec<_>>().join(", ")));
                }
            }
            Kind::Group => {
                fields.push(format!("\"group\": {}", text(self.text)));
                if self.closed {
                    fields.push("\"closed\": true".to_owned());
                }
                fields.push(format!("\"items\": [{}]", self.items.iter().map(Layout::json).collect::<Vec<_>>().join(", ")));
            }
            Kind::Button => {
                fields.push(format!("\"button\": {}", text(self.text)));
                fields.push(format!("\"message\": {}", text(self.message)));
            }
            Kind::Note => {
                fields.push(format!("\"note\": {}", text(self.text)));
                if self.warning {
                    fields.push("\"warning\": true".to_owned());
                }
            }
        }
        if !self.hint.is_empty() && matches!(self.kind, Kind::Property(_) | Kind::Button) {
            fields.push(format!("\"hint\": {}", text(self.hint)));
        }
        if let Some((property, value)) = self.when {
            let value = match value {
                Is::Number(v) => number(v),
                Is::Whole(v) => v.to_string(),
                Is::Flag(v) => v.to_string(),
                Is::Text(t) => text(t),
            };
            fields.push(format!("\"showIf\": [{}, {value}]", text(property)));
        }
        format!("{{{}}}", fields.join(", "))
    }
}
