//! Input actions: named controls (`Player/Jump`, `Player/MoveForward`) in maps, one of them active at
//! a time, bound to keys, mouse buttons and gamepad
//! buttons and sticks, so scripts ask for the action rather than the key and players can rebind.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use bayleaf_core::StableId;

use crate::{Input, PAD_AXES, PAD_BUTTONS};

/// How far a stick must lean before it counts.
pub const STICK_DEAD_ZONE: f32 = 0.15;
/// Most actions in a project, and most bindings per action.
pub const MAX_ACTIONS: usize = 128;
pub const MAX_BINDINGS: usize = 16;
/// Most maps in an Input Map.
pub const MAX_MAPS: usize = 32;

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

/// How an action reports itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActionKind {
    /// 1 in the one step it goes down, else 0: a jump, a shot.
    Trigger,
    /// 1 from the step it goes down for as long as it is held, else 0.
    #[default]
    Held,
    /// How far it is pressed, 0 to 1 (a stick or trigger leans part way): walking, steering.
    Value,
}

/// One action of an input map: `map/action`, its kind and what presses it.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub map: String,
    pub name: String,
    pub kind: ActionKind,
    pub bindings: Vec<Binding>,
}

impl Action {
    /// `map/action`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.map, self.name)
    }

    /// The strongest of its bindings, 0 to 1.
    fn raw(&self, input: &Input) -> f32 {
        self.bindings.iter().map(|b| b.value(input)).fold(0.0, f32::max)
    }
}

/// Whether `name` is 1 to 64 letters, digits or `_`.
fn is_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// An action as written: its name, its kind, and its bindings as text (`key:W`).
pub type ActionSpec = (String, ActionKind, Vec<String>);
/// A map as written: its name and its actions.
pub type MapSpec = (String, Vec<ActionSpec>);

/// The actions of an Input Map asset, checked and sorted into maps. One map is active at a time;
/// `start` is the one a game starts with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputMaps {
    actions: Vec<Action>,
    maps: Vec<String>,
    start: String,
}

impl InputMaps {
    /// Maps, each named and holding its actions (name, kind, binding texts). `start` names the
    /// first active map; empty for the first map. Refuses bad or repeated names, unknown bindings,
    /// and too many of any.
    pub fn new(list: &[MapSpec], start: &str) -> Result<Self, String> {
        if list.len() > MAX_MAPS {
            return Err(format!("an Input Map may have at most {MAX_MAPS} maps"));
        }
        if list.iter().map(|(_, actions)| actions.len()).sum::<usize>() > MAX_ACTIONS {
            return Err(format!("an Input Map may have at most {MAX_ACTIONS} actions"));
        }
        let mut actions = Vec::new();
        let mut maps: Vec<String> = Vec::new();
        for (map, list) in list {
            if !is_name(map) {
                return Err(format!("map name {map:?} must be 1 to 64 letters, digits or _"));
            }
            if maps.contains(map) {
                return Err(format!("map {map} is named twice"));
            }
            maps.push(map.clone());
            let mut seen = HashSet::new();
            for (name, kind, bindings) in list {
                if !is_name(name) {
                    return Err(format!("action name {name:?} in {map} must be 1 to 64 letters, digits or _"));
                }
                if !seen.insert(name.clone()) {
                    return Err(format!("action {map}/{name} is named twice"));
                }
                if bindings.len() > MAX_BINDINGS {
                    return Err(format!("action {map}/{name} may have at most {MAX_BINDINGS} bindings"));
                }
                let parsed = bindings.iter().map(|b| Binding::parse(b)).collect::<Result<Vec<_>, _>>().map_err(|e| format!("action {map}/{name}: {e}"))?;
                actions.push(Action { map: map.clone(), name: name.clone(), kind: *kind, bindings: parsed });
            }
        }
        let start = if start.is_empty() { maps.first().cloned().unwrap_or_default() } else { start.to_owned() };
        if !maps.is_empty() && !maps.contains(&start) {
            return Err(format!("there is no map {start} to start with; the maps are {}", maps.join(", ")));
        }
        Ok(Self { actions, maps, start })
    }

    /// The maps, in their order.
    pub fn maps(&self) -> &[String] {
        &self.maps
    }

    /// The map that is active when play starts.
    pub fn start(&self) -> &str {
        &self.start
    }

    pub fn actions(&self) -> &[Action] {
        &self.actions
    }

    /// The action `name` means while `active` is the active map: `map/action` when `map` is
    /// active, or a plain `action` of the active map. Actions of other maps are not active.
    fn find(&self, active: &str, name: &str) -> Option<&Action> {
        let (map, action) = name.split_once('/').unwrap_or((active, name));
        if map != active {
            return None;
        }
        self.actions.iter().find(|a| a.map == map && a.name == action)
    }
}

/// An Input Map in play: which map is active, and which actions were down in the last step and
/// this one, so a trigger counts only in the step it goes down.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputState {
    maps: Arc<InputMaps>,
    active: String,
    down_before: HashSet<String>,
    down_now: HashSet<String>,
}

impl InputState {
    pub fn new(maps: Arc<InputMaps>) -> Self {
        let active = maps.start().to_owned();
        Self { maps, active, down_before: HashSet::new(), down_now: HashSet::new() }
    }

    pub fn maps(&self) -> &InputMaps {
        &self.maps
    }

    /// The active map.
    pub fn active(&self) -> &str {
        &self.active
    }

    /// Makes `map` the active map; false when the asset has no such map.
    pub fn set_active(&mut self, map: &str) -> bool {
        let known = self.maps.maps.iter().any(|m| m == map);
        if known {
            self.active = map.to_owned();
        }
        known
    }

    /// Takes the step's input: which actions of every map are down now. Every map is tracked, so
    /// an action held while its map becomes active does not count as going down then.
    pub fn step(&mut self, input: &Input) {
        self.down_before = std::mem::take(&mut self.down_now);
        self.down_now = self.maps.actions.iter().filter(|a| a.raw(input) > 0.5).map(Action::full_name).collect();
    }

    /// The action's value now, by its kind: a trigger 1 in the step it went down, held 1 while
    /// down, value how far it is pressed. 0 for an action of an inactive map or an unknown one.
    pub fn value(&self, name: &str, input: &Input) -> f32 {
        let Some(action) = self.maps.find(&self.active, name) else { return 0.0 };
        let full = action.full_name();
        match action.kind {
            ActionKind::Value => action.raw(input),
            ActionKind::Held => f32::from(u8::from(self.down_now.contains(&full))),
            ActionKind::Trigger => f32::from(u8::from(self.down_now.contains(&full) && !self.down_before.contains(&full))),
        }
    }

    /// True in the step the action went down, whatever its kind.
    pub fn pressed(&self, name: &str) -> bool {
        self.maps.find(&self.active, name).is_some_and(|a| {
            let full = a.full_name();
            self.down_now.contains(&full) && !self.down_before.contains(&full)
        })
    }
}

/// The Input Maps scripts read in play: the project's, and objects' own (an Input Map component).
#[derive(Clone, Debug, Default)]
pub struct InputSet {
    pub project: Option<InputState>,
    pub objects: HashMap<StableId, InputState>,
}

impl InputSet {
    /// What an object's scripts read: its own Input Map, else the project's.
    pub fn of(&self, object: StableId) -> Option<&InputState> {
        self.objects.get(&object).or(self.project.as_ref())
    }

    /// The project's map (`None`) or an object's own.
    pub fn get_mut(&mut self, object: Option<StableId>) -> Option<&mut InputState> {
        match object {
            Some(id) => self.objects.get_mut(&id),
            None => self.project.as_mut(),
        }
    }

    /// Takes the step's input in every map.
    pub fn step(&mut self, input: &Input) {
        for state in self.project.iter_mut().chain(self.objects.values_mut()) {
            state.step(input);
        }
    }
}

/// The input maps every script of a play session shares; switching a map is seen by all.
pub type SharedInput = Arc<Mutex<InputSet>>;

/// The input maps, also after a script panicked while holding them.
pub fn lock(input: &SharedInput) -> std::sync::MutexGuard<'_, InputSet> {
    input.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ActionKind, Binding, InputMaps, InputState};
    use crate::{Input, Pad};

    fn maps() -> InputMaps {
        let action = |name: &str, kind, bindings: &[&str]| (name.to_owned(), kind, bindings.iter().map(|b| (*b).to_owned()).collect());
        InputMaps::new(
            &[
                (
                    "Player".to_owned(),
                    vec![
                        action("MoveRight", ActionKind::Value, &["key:D", "axis:LeftX+"]),
                        action("Jump", ActionKind::Trigger, &["key:Space", "pad:South", "mouse:Right"]),
                        action("Fire", ActionKind::Held, &["key:F"]),
                    ],
                ),
                ("Menu".to_owned(), vec![action("Confirm", ActionKind::Trigger, &["key:Space"])]),
            ],
            "",
        )
        .unwrap()
    }

    fn keys(names: &[&str]) -> Input {
        Input { keys: names.iter().map(|k| (*k).to_owned()).collect(), ..Input::default() }
    }

    #[test]
    fn bindings_read_keys_buttons_and_sticks_past_the_dead_zone() {
        let mut state = InputState::new(Arc::new(maps()));
        let lean = |x: f32| Input { pad: Some(Pad { axes: [x, 0.0, 0.0, 0.0, 0.0, 0.0], buttons: 0 }), ..Input::default() };
        state.step(&lean(0.1));
        assert_eq!(state.value("MoveRight", &lean(0.1)), 0.0, "inside the dead zone");
        assert!(state.value("MoveRight", &lean(0.6)) > 0.4 && state.value("MoveRight", &lean(-0.6)) == 0.0, "only the bound direction");
        assert!(Binding::parse("pad:Jump").is_err());
        assert!(Binding::parse("axis:LeftX").is_err(), "a direction is needed");
    }

    #[test]
    fn a_trigger_counts_once_held_while_down_and_only_the_active_map_answers() {
        let mut state = InputState::new(Arc::new(maps()));
        assert_eq!(state.active(), "Player", "the first map starts");
        let space = keys(&["Space", "F"]);
        state.step(&space);
        assert_eq!(state.value("Player/Jump", &space), 1.0, "a trigger in the step it goes down");
        assert_eq!(state.value("Fire", &space), 1.0, "a plain name is in the active map");
        assert_eq!(state.value("Menu/Confirm", &space), 0.0, "another map's action is not active");
        state.step(&space);
        assert_eq!(state.value("Jump", &space), 0.0, "and not while it stays down");
        assert_eq!(state.value("Fire", &space), 1.0, "held stays on");

        assert!(state.set_active("Menu") && !state.set_active("Nowhere"));
        state.step(&space);
        assert_eq!(state.value("Confirm", &space), 0.0, "held from before the switch, it does not go down now");
        assert_eq!(state.value("Player/Jump", &space), 0.0);
        let idle = Input::default();
        state.step(&idle);
        state.step(&space);
        assert!(state.pressed("Confirm") && state.value("Confirm", &space) == 1.0);
    }

    #[test]
    fn maps_hold_their_own_actions_names_are_unique_and_the_start_map_exists() {
        let map = |map: &str, actions: &[&str]| (map.to_owned(), actions.iter().map(|a| ((*a).to_owned(), ActionKind::Held, Vec::new())).collect::<Vec<_>>());
        assert!(InputMaps::new(&[map("Player", &["Jump/High"])], "").is_err(), "an action name is one name");
        assert!(InputMaps::new(&[map("Pla yer", &["Jump"])], "").is_err());
        assert!(InputMaps::new(&[map("A", &["B", "B"])], "").is_err(), "names are unique in a map");
        assert!(InputMaps::new(&[map("A", &["B"]), map("A", &[])], "").is_err(), "and maps are unique");
        let both = InputMaps::new(&[map("A", &["Jump"]), map("B", &["Jump"])], "B").unwrap();
        assert_eq!(both.start(), "B", "two maps may each have a Jump");
        assert!(InputMaps::new(&[map("A", &["B"])], "C").is_err(), "the starting map exists");
        assert_eq!(InputMaps::new(&[map("A", &["B"]), map("Empty", &[])], "").unwrap().maps(), ["A", "Empty"], "a map may be empty for now");
    }
}
