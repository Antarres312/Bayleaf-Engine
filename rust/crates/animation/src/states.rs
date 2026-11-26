//! Animation states (`.banim`, domain `animation`): a graph of states, each a motion (a clip, a
//! mix or a layer), and transitions between them on conditions over parameters that scripts set.
//! [`compile`] turns a graph into a [`Machine`]; a [`Player`](crate::Player) runs it on the
//! object's base layer (see `docs/graphs.md`).

use bayleaf_graph::{Catalog, Graph, NodeDef, PinDef, PinDefault, PinType, Problem, SettingDef, SettingKind};

use crate::motion::{Motion, Number, Parameter};

use std::collections::BTreeMap;

const fn pin(name: &'static str, label: &'static str, ty: PinType) -> PinDef {
    PinDef::new(name, label, ty)
}

const fn number(name: &'static str, label: &'static str, value: f32) -> PinDef {
    PinDef::with(name, label, PinType::Float, PinDefault::Float(value))
}

const MOTION_OUT: &[PinDef] = &[pin("motion", "Motion", PinType::Motion)];
const BOOL_OUT: &[PinDef] = &[pin("result", "Result", PinType::Bool)];
const NAME: &[SettingDef] = &[SettingDef { name: "name", label: "Name", kind: SettingKind::Name("") }];

/// The node kinds of animation states.
pub const CATALOG: Catalog = Catalog {
    domain: "animation",
    label: "Animation States",
    extension: "banim",
    starter: &[("entry", [40.0, 40.0]), ("state", [40.0, 140.0]), ("clip", [-220.0, 140.0])],
    variant: bayleaf_graph::no_variant,
    upgrade: bayleaf_graph::no_upgrade,
    fractional: &[],
    nodes: &[
        NodeDef {
            kind: "entry",
            label: "Start State",
            category: "States",
            description: "The state an object starts in. Without one, it starts in the first state.",
            inputs: &[],
            outputs: &[],
            settings: &[SettingDef { name: "state", label: "State", kind: SettingKind::Name("Idle") }],
            unique: true,
        },
        NodeDef {
            kind: "state",
            label: "State",
            category: "States",
            description: "A state the object can be in, by its name, playing the motion linked in (at rest when none is).",
            inputs: &[pin("motion", "Motion", PinType::Motion)],
            outputs: &[],
            settings: &[SettingDef { name: "name", label: "Name", kind: SettingKind::Name("Idle") }],
            unique: false,
        },
        NodeDef {
            kind: "transition",
            label: "Transition",
            category: "States",
            description: "Goes from one state (or, with From left empty, from any other) to another while When is true, fading over Fade seconds. Transitions are tried in the order they were placed.",
            inputs: &[PinDef::with("when", "When", PinType::Bool, PinDefault::Bool(false))],
            outputs: &[],
            settings: &[
                SettingDef { name: "from", label: "From", kind: SettingKind::Name("") },
                SettingDef { name: "to", label: "To", kind: SettingKind::Name("") },
                SettingDef { name: "fade", label: "Fade", kind: SettingKind::Float(0.25) },
            ],
            unique: false,
        },
        NodeDef {
            kind: "clip",
            label: "Clip",
            category: "Motions",
            description: "One of the model's clips by name, at a speed, looping or once.",
            inputs: &[],
            outputs: MOTION_OUT,
            settings: &[
                SettingDef { name: "clip", label: "Clip", kind: SettingKind::Text("") },
                SettingDef { name: "speed", label: "Speed", kind: SettingKind::Float(1.0) },
                SettingDef { name: "play", label: "Play", kind: SettingKind::Choice(&["Loop", "Once"]) },
            ],
            unique: false,
        },
        NodeDef {
            kind: "mix",
            label: "Mix",
            category: "Motions",
            description: "A and B by where By lies from At A to At B (walk and run by speed, aiming up and down by angle); before At A all A, past At B all B. Both play in step. Link a Mix into A or B to mix more.",
            inputs: &[pin("a", "A", PinType::Motion), pin("b", "B", PinType::Motion), number("by", "By", 0.0)],
            outputs: MOTION_OUT,
            settings: &[
                SettingDef { name: "atA", label: "At A", kind: SettingKind::Float(0.0) },
                SettingDef { name: "atB", label: "At B", kind: SettingKind::Float(1.0) },
            ],
            unique: false,
        },
        NodeDef {
            kind: "layer",
            label: "Layer",
            category: "Motions",
            description: "Top over Base on the joint named Joint and every joint under it (a wave on the arm while the legs walk), by Weight from 0 to 1.",
            inputs: &[pin("base", "Base", PinType::Motion), pin("top", "Top", PinType::Motion), number("weight", "Weight", 1.0)],
            outputs: MOTION_OUT,
            settings: &[SettingDef { name: "joint", label: "Joint", kind: SettingKind::Text("") }],
            unique: false,
        },
        NodeDef {
            kind: "number",
            label: "Number Parameter",
            category: "Parameters",
            description: "A number parameter scripts set (0 until one does).",
            inputs: &[],
            outputs: &[pin("value", "Value", PinType::Float)],
            settings: NAME,
            unique: false,
        },
        NodeDef {
            kind: "flag",
            label: "True/False Parameter",
            category: "Parameters",
            description: "A true/false parameter scripts set (false until one does).",
            inputs: &[],
            outputs: &[pin("value", "Value", PinType::Bool)],
            settings: NAME,
            unique: false,
        },
        NodeDef {
            kind: "trigger",
            label: "Trigger Parameter",
            category: "Parameters",
            description: "True once a script triggers it, until a transition it lets through uses it up.",
            inputs: &[],
            outputs: &[pin("value", "Value", PinType::Bool)],
            settings: NAME,
            unique: false,
        },
        NodeDef {
            kind: "stateTime",
            label: "Time in State",
            category: "Conditions",
            description: "Seconds since the state began, and Runs: how many times its motion has played through (1 at the end of its first run).",
            inputs: &[],
            outputs: &[pin("seconds", "Seconds", PinType::Float), pin("runs", "Runs", PinType::Float)],
            settings: &[],
            unique: false,
        },
        NodeDef {
            kind: "compare",
            label: "Compare",
            category: "Conditions",
            description: "Whether A is greater than, less than, at least, at most, equal to or not equal to B.",
            inputs: &[number("a", "A", 0.0), number("b", "B", 0.0)],
            outputs: BOOL_OUT,
            settings: &[SettingDef {
                name: "test",
                label: "Test",
                kind: SettingKind::Choice(&["Greater", "Less", "At Least", "At Most", "Equal", "Not Equal"]),
            }],
            unique: false,
        },
        NodeDef {
            kind: "and",
            label: "And",
            category: "Conditions",
            description: "True when A and B both are.",
            inputs: &[PinDef::with("a", "A", PinType::Bool, PinDefault::Bool(true)), PinDef::with("b", "B", PinType::Bool, PinDefault::Bool(true))],
            outputs: BOOL_OUT,
            settings: &[],
            unique: false,
        },
        NodeDef {
            kind: "or",
            label: "Or",
            category: "Conditions",
            description: "True when A or B is.",
            inputs: &[PinDef::with("a", "A", PinType::Bool, PinDefault::Bool(false)), PinDef::with("b", "B", PinType::Bool, PinDefault::Bool(false))],
            outputs: BOOL_OUT,
            settings: &[],
            unique: false,
        },
        NodeDef {
            kind: "not",
            label: "Not",
            category: "Conditions",
            description: "True when In is false.",
            inputs: &[PinDef::with("in", "In", PinType::Bool, PinDefault::Bool(false))],
            outputs: BOOL_OUT,
            settings: &[],
            unique: false,
        },
    ],
};

/// What kind of value a parameter holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterKind {
    Number,
    Flag,
    Trigger,
}

impl ParameterKind {
    pub fn label(self) -> &'static str {
        match self {
            ParameterKind::Number => "number",
            ParameterKind::Flag => "true/false",
            ParameterKind::Trigger => "trigger",
        }
    }
}

/// A number a condition reads.
#[derive(Clone, Debug, PartialEq)]
pub enum Amount {
    Value(f32),
    Parameter(String),
    /// Seconds since the state began.
    Seconds,
    /// Times the state's motion has played through.
    Runs,
}

/// How [`Condition::Compare`] compares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Test {
    Greater,
    Less,
    AtLeast,
    AtMost,
    Equal,
    NotEqual,
}

/// When a transition goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    Value(bool),
    Flag(String),
    Trigger(String),
    Compare(Amount, Test, Amount),
    And(Box<Condition>, Box<Condition>),
    Or(Box<Condition>, Box<Condition>),
    Not(Box<Condition>),
}

/// What a condition reads besides the parameters: the state's time.
#[derive(Clone, Copy, Debug, Default)]
pub struct InState {
    pub seconds: f32,
    pub runs: f32,
}

impl Amount {
    fn read(&self, parameters: &BTreeMap<String, Parameter>, state: InState) -> f32 {
        match self {
            Amount::Value(v) => *v,
            Amount::Parameter(name) => parameters.get(name).map_or(0.0, Parameter::number),
            Amount::Seconds => state.seconds,
            Amount::Runs => state.runs,
        }
    }
}

impl Condition {
    /// Whether it holds now.
    pub fn holds(&self, parameters: &BTreeMap<String, Parameter>, state: InState) -> bool {
        match self {
            Condition::Value(b) => *b,
            Condition::Flag(name) | Condition::Trigger(name) => parameters.get(name).is_some_and(Parameter::flag),
            Condition::Compare(a, test, b) => {
                let (a, b) = (a.read(parameters, state), b.read(parameters, state));
                match test {
                    Test::Greater => a > b,
                    Test::Less => a < b,
                    Test::AtLeast => a >= b,
                    Test::AtMost => a <= b,
                    Test::Equal => (a - b).abs() <= f32::EPSILON * a.abs().max(b.abs()).max(1.0),
                    Test::NotEqual => (a - b).abs() > f32::EPSILON * a.abs().max(b.abs()).max(1.0),
                }
            }
            Condition::And(a, b) => a.holds(parameters, state) && b.holds(parameters, state),
            Condition::Or(a, b) => a.holds(parameters, state) || b.holds(parameters, state),
            Condition::Not(c) => !c.holds(parameters, state),
        }
    }

    /// The triggers it reads, which go when a transition it lets through is taken.
    pub fn triggers(&self, out: &mut Vec<String>) {
        match self {
            Condition::Trigger(name) => out.push(name.clone()),
            Condition::And(a, b) | Condition::Or(a, b) => {
                a.triggers(out);
                b.triggers(out);
            }
            Condition::Not(c) => c.triggers(out),
            Condition::Value(_) | Condition::Flag(_) | Condition::Compare(..) => {}
        }
    }
}

/// A state: its name and what it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub name: String,
    pub motion: Motion,
}

/// A way from one state (or any) to another.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    /// `None` from any other state.
    pub from: Option<usize>,
    pub to: usize,
    /// Seconds the fade takes.
    pub fade: f32,
    pub when: Condition,
}

/// Compiled animation states.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub states: Vec<State>,
    pub start: usize,
    /// In the order they are tried.
    pub transitions: Vec<Transition>,
    /// Every parameter the graph reads, by name, and its kind.
    pub parameters: BTreeMap<String, ParameterKind>,
}

impl Machine {
    pub fn state(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name == name)
    }

    /// The transition to take from state `current` now, if any: the first whose condition holds
    /// (one from any state never goes to the state it is in).
    pub fn next(&self, current: usize, parameters: &BTreeMap<String, Parameter>, state: InState) -> Option<&Transition> {
        self.transitions.iter().find(|t| t.from.is_none_or(|f| f == current) && !(t.from.is_none() && t.to == current) && t.when.holds(parameters, state))
    }
}

struct Compiler<'a> {
    graph: &'a Graph,
    problems: Vec<Problem>,
    parameters: BTreeMap<String, ParameterKind>,
}

impl Compiler<'_> {
    fn text(&self, node: u32, name: &str, default: &str) -> String {
        self.graph.value(node, name).and_then(|v| v.as_str()).unwrap_or(default).to_owned()
    }

    fn float(&self, node: u32, name: &str, default: f32) -> f32 {
        self.graph.value(node, name).and_then(bayleaf_graph::as_f32).filter(|v| v.is_finite()).unwrap_or(default)
    }

    fn bool(&self, node: u32, name: &str, default: bool) -> bool {
        self.graph.value(node, name).and_then(serde_json::Value::as_bool).unwrap_or(default)
    }

    fn kind(&self, node: u32) -> &str {
        self.graph.node(node).map_or("", |n| n.kind.as_str())
    }

    fn label(&self, node: u32) -> &'static str {
        CATALOG.get(self.kind(node)).map_or("Node", |d| d.label)
    }

    /// The node and output linked into `node`'s input `pin`.
    fn source(&self, node: u32, pin: &str) -> Option<(u32, String)> {
        self.graph.link_into(node, pin).map(|l| (l.from.node, l.from.pin.clone()))
    }

    fn parameter(&mut self, node: u32, kind: ParameterKind) -> String {
        let name = self.text(node, "name", "");
        if name.is_empty() {
            self.problems.push(Problem::on(node, format!("{} · give the parameter a name", self.label(node))));
            return name;
        }
        match self.parameters.get(&name) {
            Some(other) if *other != kind => self.problems.push(Problem::on(
                node,
                format!("{} · \"{name}\" is a {} parameter elsewhere in the graph; one name holds one kind", self.label(node), other.label()),
            )),
            _ => {
                self.parameters.insert(name.clone(), kind);
            }
        }
        name
    }

    /// The motion linked into `node`'s input `pin`; at rest when nothing is.
    fn motion_into(&mut self, node: u32, pin: &str, depth: usize) -> Motion {
        match self.source(node, pin) {
            Some((from, _)) => self.motion(from, depth + 1),
            None => Motion::Rest,
        }
    }

    fn motion(&mut self, node: u32, depth: usize) -> Motion {
        if depth > 64 {
            self.problems.push(Problem::on(node, "motions are nested more than 64 deep"));
            return Motion::Rest;
        }
        match self.kind(node) {
            "clip" => {
                let clip = self.text(node, "clip", "");
                if clip.is_empty() {
                    self.problems.push(Problem::on(node, "Clip · name one of the model's clips"));
                }
                let speed = self.float(node, "speed", 1.0);
                if !(0.0..=10.0).contains(&speed) {
                    self.problems.push(Problem::on(node, "Clip · speed must be from 0 to 10"));
                }
                Motion::Clip { clip, speed: speed.clamp(0.0, 10.0), looping: self.text(node, "play", "Loop") != "Once" }
            }
            "mix" => {
                let (a, b) = (self.motion_into(node, "a", depth), self.motion_into(node, "b", depth));
                if self.source(node, "a").is_none() || self.source(node, "b").is_none() {
                    self.problems.push(Problem::on(node, "Mix · link a motion into both A and B"));
                }
                let by = self.number_into(node, "by", 0.0);
                Motion::Mix { items: vec![(a, self.float(node, "atA", 0.0)), (b, self.float(node, "atB", 1.0))], by }
            }
            "layer" => {
                let (base, top) = (self.motion_into(node, "base", depth), self.motion_into(node, "top", depth));
                if self.source(node, "top").is_none() {
                    self.problems.push(Problem::on(node, "Layer · link the motion it plays into Top"));
                }
                let joint = self.text(node, "joint", "");
                let weight = self.number_into(node, "weight", 1.0);
                Motion::Layer { base: Box::new(base), top: Box::new(top), joint, weight }
            }
            _ => {
                self.problems.push(Problem::on(node, format!("{} gives no motion", self.label(node))));
                Motion::Rest
            }
        }
    }

    /// A number a motion reads, linked into `pin` (a number parameter) or typed in.
    fn number_into(&mut self, node: u32, pin: &str, default: f32) -> Number {
        match self.source(node, pin) {
            None => Number::Value(self.float(node, pin, default)),
            Some((from, _)) if self.kind(from) == "number" => Number::Parameter(self.parameter(from, ParameterKind::Number)),
            Some((from, _)) => {
                self.problems.push(Problem::on(
                    node,
                    format!("{} · a motion reads a number parameter or a number typed in, not {}", self.label(node), self.label(from)),
                ));
                Number::Value(default)
            }
        }
    }

    /// A number a condition reads.
    fn amount_into(&mut self, node: u32, pin: &str) -> Amount {
        match self.source(node, pin) {
            None => Amount::Value(self.float(node, pin, 0.0)),
            Some((from, out)) => match self.kind(from) {
                "number" => Amount::Parameter(self.parameter(from, ParameterKind::Number)),
                "stateTime" if out == "runs" => Amount::Runs,
                "stateTime" => Amount::Seconds,
                _ => {
                    self.problems.push(Problem::on(node, format!("{} · cannot read a number from {}", self.label(node), self.label(from))));
                    Amount::Value(0.0)
                }
            },
        }
    }

    fn condition_into(&mut self, node: u32, pin: &str, default: bool, depth: usize) -> Condition {
        match self.source(node, pin) {
            None => Condition::Value(self.bool(node, pin, default)),
            Some((from, _)) => self.condition(from, depth + 1),
        }
    }

    fn condition(&mut self, node: u32, depth: usize) -> Condition {
        if depth > 64 {
            self.problems.push(Problem::on(node, "conditions are nested more than 64 deep"));
            return Condition::Value(false);
        }
        match self.kind(node) {
            "flag" => Condition::Flag(self.parameter(node, ParameterKind::Flag)),
            "trigger" => Condition::Trigger(self.parameter(node, ParameterKind::Trigger)),
            "compare" => {
                let test = match self.text(node, "test", "Greater").as_str() {
                    "Less" => Test::Less,
                    "At Least" => Test::AtLeast,
                    "At Most" => Test::AtMost,
                    "Equal" => Test::Equal,
                    "Not Equal" => Test::NotEqual,
                    _ => Test::Greater,
                };
                Condition::Compare(self.amount_into(node, "a"), test, self.amount_into(node, "b"))
            }
            "and" => Condition::And(Box::new(self.condition_into(node, "a", true, depth)), Box::new(self.condition_into(node, "b", true, depth))),
            "or" => Condition::Or(Box::new(self.condition_into(node, "a", false, depth)), Box::new(self.condition_into(node, "b", false, depth))),
            "not" => Condition::Not(Box::new(self.condition_into(node, "in", false, depth))),
            _ => {
                self.problems.push(Problem::on(node, format!("{} gives no condition", self.label(node))));
                Condition::Value(false)
            }
        }
    }
}

/// Compiles animation states; every problem found, each on its node where it has one.
pub fn compile(graph: &Graph) -> Result<Machine, Vec<Problem>> {
    if graph.domain != CATALOG.domain {
        return Err(vec![Problem::graph(format!("this is a {} graph, not animation states", graph.domain))]);
    }
    let mut c = Compiler { graph, problems: Vec::new(), parameters: BTreeMap::new() };
    let mut states: Vec<State> = Vec::new();
    for node in graph.nodes.iter().filter(|n| n.kind == "state") {
        let name = c.text(node.id, "name", "Idle");
        if name.is_empty() {
            c.problems.push(Problem::on(node.id, "State · give the state a name"));
        } else if states.iter().any(|s| s.name == name) {
            c.problems.push(Problem::on(node.id, format!("State · another state is named \"{name}\"")));
        }
        let motion = c.motion_into(node.id, "motion", 0);
        states.push(State { name, motion });
    }
    if states.is_empty() {
        c.problems.push(Problem::graph("add a State: animation states need one to start in"));
    }
    let find = |name: &str| states.iter().position(|s| s.name == name);
    let mut start = 0;
    if let Some(entry) = graph.nodes.iter().find(|n| n.kind == "entry") {
        let name = c.text(entry.id, "state", "Idle");
        match find(&name) {
            Some(i) => start = i,
            None => c.problems.push(Problem::on(entry.id, format!("Start State · there is no state named \"{name}\""))),
        }
    }
    let mut transitions = Vec::new();
    for node in graph.nodes.iter().filter(|n| n.kind == "transition") {
        let (from, to) = (c.text(node.id, "from", ""), c.text(node.id, "to", ""));
        let from_index = if from.is_empty() { None } else { find(&from) };
        if !from.is_empty() && from_index.is_none() {
            c.problems.push(Problem::on(node.id, format!("Transition · there is no state named \"{from}\" to go from")));
        }
        let Some(to_index) = find(&to) else {
            c.problems.push(Problem::on(
                node.id,
                if to.is_empty() {
                    "Transition · name the state it goes to".to_owned()
                } else {
                    format!("Transition · there is no state named \"{to}\" to go to")
                },
            ));
            continue;
        };
        let fade = c.float(node.id, "fade", 0.25);
        if !(0.0..=crate::player::MAX_FADE).contains(&fade) {
            c.problems.push(Problem::on(node.id, format!("Transition · fade must be from 0 to {} seconds", crate::player::MAX_FADE)));
        }
        let when = c.condition_into(node.id, "when", false, 0);
        if !from.is_empty() && from_index.is_none() {
            continue;
        }
        transitions.push(Transition { from: from_index, to: to_index, fade: fade.clamp(0.0, crate::player::MAX_FADE), when });
    }
    // Parameter nodes no condition or motion reads still declare their parameter.
    for node in &graph.nodes {
        match node.kind.as_str() {
            "number" => {
                c.parameter(node.id, ParameterKind::Number);
            }
            "flag" => {
                c.parameter(node.id, ParameterKind::Flag);
            }
            "trigger" => {
                c.parameter(node.id, ParameterKind::Trigger);
            }
            _ => {}
        }
    }
    if c.problems.is_empty() {
        Ok(Machine { states, start, transitions, parameters: c.parameters })
    } else {
        let mut unique: Vec<Problem> = Vec::new();
        for problem in c.problems {
            if !unique.contains(&problem) {
                unique.push(problem);
            }
        }
        Err(unique)
    }
}
