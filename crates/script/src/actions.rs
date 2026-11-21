//! Input actions: named controls ("Jump", "MoveForward") bound to keys, mouse buttons and gamepad
//! buttons and sticks, so scripts ask for the action rather than the key and players can rebind.

use std::collections::HashSet;

use crate::{Input, PAD_AXES, PAD_BUTTONS};

/// How far a stick must lean before it counts.
pub const STICK_DEAD_ZONE: f32 = 0.15;
/// Most actions in a project, and most bindings per action.
pub const MAX_ACTIONS: usize = 128;
pub const MAX_BINDINGS: usize = 16;

/// One way to press an action.
#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    /// A keyboard key by name, as in `docs/script-abi.md` (`W`, `Space`, `ArrowUp`).
    Key(String),
    /// Mouse button 0 left, 1 right, 2 middle.
    Mouse(u8),
    /// A gamepad button by index in [`PAD_BUTTONS`].
    Pad(u8),
    /// A gamepad axis by index in [`PAD_AXES`], counting one direction: `true` positive.
    Axis(u8, bool),
}

impl Binding {
    /// Reads `key:W`, `mouse:Left`, `pad:South` or `axis:LeftX+` / `axis:LeftY-`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (kind, name) = text.split_once(':').ok_or_else(|| format!("{text:?}: write key:NAME, mouse:Left, pad:BUTTON or axis:AXIS+ / axis:AXIS-"))?;
        match kind {
            "key" if !name.is_empty() && name.len() <= 32 => Ok(Binding::Key(name.to_owned())),
            "mouse" => match name {
                "Left" => Ok(Binding::Mouse(0)),
                "Right" => Ok(Binding::Mouse(1)),
                "Middle" => Ok(Binding::Mouse(2)),
                _ => Err(format!("{text:?}: the mouse buttons are Left, Right and Middle")),
            },
            "pad" => PAD_BUTTONS
                .iter()
                .position(|b| *b == name)
                .map(|i| Binding::Pad(i as u8))
                .ok_or_else(|| format!("{text:?}: gamepad buttons are {}", PAD_BUTTONS.join(", "))),
            "axis" => {
                let (axis, positive) = match name.strip_suffix('+') {
                    Some(axis) => (axis, true),
                    None => (name.strip_suffix('-').ok_or_else(|| format!("{text:?}: end an axis with + or -"))?, false),
                };
                PAD_AXES
                    .iter()
                    .position(|a| *a == axis)
                    .map(|i| Binding::Axis(i as u8, positive))
                    .ok_or_else(|| format!("{text:?}: gamepad axes are {}", PAD_AXES.join(", ")))
            }
            _ => Err(format!("{text:?}: write key:NAME, mouse:Left, pad:BUTTON or axis:AXIS+ / axis:AXIS-")),
        }
    }

    /// How far this binding is pressed now, from 0 to 1.
    pub fn value(&self, input: &Input) -> f32 {
        match self {
            Binding::Key(name) => f32::from(u8::from(input.keys.contains(name))),
            Binding::Mouse(b) => f32::from((input.buttons >> b) & 1),
            Binding::Pad(b) => input.pad.map_or(0.0, |p| f32::from(u8::from((p.buttons >> b) & 1 == 1))),
            Binding::Axis(a, positive) => input.pad.map_or(0.0, |p| {
                let v = p.axes.get(usize::from(*a)).copied().unwrap_or(0.0);
                let v = if *positive { v } else { -v };
                if v < STICK_DEAD_ZONE { 0.0 } else { ((v - STICK_DEAD_ZONE) / (1.0 - STICK_DEAD_ZONE)).min(1.0) }
            }),
        }
    }
}

/// A project's actions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Actions {
    actions: Vec<(String, Vec<Binding>)>,
}

impl Actions {
    /// Actions from names and binding texts; refuses unknown bindings and too many of either.
    pub fn new(list: &[(String, Vec<String>)]) -> Result<Self, String> {
        if list.len() > MAX_ACTIONS {
            return Err(format!("a project may have at most {MAX_ACTIONS} actions"));
        }
        let mut names = HashSet::new();
        let mut actions = Vec::new();
        for (name, bindings) in list {
            if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!("action name {name:?} must be 1 to 64 letters, digits or _"));
            }
            if !names.insert(name.clone()) {
                return Err(format!("action {name} is named twice"));
            }
            if bindings.len() > MAX_BINDINGS {
                return Err(format!("action {name} may have at most {MAX_BINDINGS} bindings"));
            }
            let parsed = bindings.iter().map(|b| Binding::parse(b)).collect::<Result<Vec<_>, _>>().map_err(|e| format!("action {name}: {e}"))?;
            actions.push((name.clone(), parsed));
        }
        Ok(Self { actions })
    }

    /// How far the action is pressed, 0 to 1 (the strongest of its bindings); 0 for an unknown action.
    pub fn value(&self, name: &str, input: &Input) -> f32 {
        self.actions.iter().find(|(n, _)| n == name).map_or(0.0, |(_, bindings)| bindings.iter().map(|b| b.value(input)).fold(0.0, f32::max))
    }

    /// The action names, in the order they were given.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.actions.iter().map(|(n, _)| n.as_str())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.actions.iter().any(|(n, _)| n == name)
    }
}

#[cfg(test)]
mod tests {
    use super::{Actions, Binding};
    use crate::{Input, Pad};

    #[test]
    fn bindings_read_keys_buttons_and_sticks_past_the_dead_zone() {
        let actions = Actions::new(&[
            ("MoveRight".to_owned(), vec!["key:D".to_owned(), "axis:LeftX+".to_owned()]),
            ("Jump".to_owned(), vec!["key:Space".to_owned(), "pad:South".to_owned(), "mouse:Right".to_owned()]),
        ])
        .unwrap();
        let idle = Input::default();
        assert_eq!(actions.value("MoveRight", &idle), 0.0);
        let key = Input { keys: ["D".to_owned()].into_iter().collect(), ..Input::default() };
        assert_eq!(actions.value("MoveRight", &key), 1.0);
        let lean = |x: f32| Input { pad: Some(Pad { axes: [x, 0.0, 0.0, 0.0, 0.0, 0.0], buttons: 1 }), ..Input::default() };
        assert_eq!(actions.value("MoveRight", &lean(0.1)), 0.0, "inside the dead zone");
        assert!(actions.value("MoveRight", &lean(0.6)) > 0.4 && actions.value("MoveRight", &lean(-0.6)) == 0.0, "only the bound direction");
        assert_eq!(actions.value("Jump", &lean(0.0)), 1.0, "South is button 0");
        assert_eq!(actions.value("Jump", &Input { buttons: 2, ..Input::default() }), 1.0);
        assert_eq!(actions.value("Nothing", &key), 0.0);

        assert!(Binding::parse("pad:Jump").is_err());
        assert!(Binding::parse("axis:LeftX").is_err(), "a direction is needed");
        assert!(Actions::new(&[("A".to_owned(), vec![]), ("A".to_owned(), vec![])]).is_err(), "names are unique");
    }
}
