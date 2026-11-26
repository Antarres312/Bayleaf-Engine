//! Keyframe animations (`.btimeline`): values of a prefab's parts that change over time.
//!
//! A [`Timeline`] is made for one prefab. It lists the parts that take part (objects of the
//! prefab's hierarchy, by their ids in the prefab file) and gives each part tracks: one value each,
//! a component's field (`transform.position`, `pointLight.color`) or a script's property. A track's
//! keys hold the value at a moment and say how it eases to the next key.
//!
//! The interface other code plays a timeline through (play mode, the editor's playhead, animation
//! states) is small: [`Timeline::local_time`] maps a clock to a moment of the timeline,
//! [`Timeline::sample`] gives every track's value at that moment, and [`apply`] writes those values
//! to the objects a [`Binding`] names.

use std::collections::BTreeMap;

use bayleaf_core::StableId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Scene;
use crate::components::{PropertyValue, Scripts};

/// `format` of timeline files.
pub const FORMAT: &str = "bayleaf.timeline";
/// Version of the timeline file format.
pub const VERSION: u32 = 1;
/// File extension of timeline files.
pub const EXTENSION: &str = "btimeline";

/// Longest timeline, in seconds.
pub const MAX_LENGTH: f32 = 3600.0;
/// Most parts one timeline may animate.
pub const MAX_PARTS: usize = 1024;
/// Most tracks one part may have.
pub const MAX_TRACKS: usize = 64;
/// Most keys one track may have.
pub const MAX_KEYS: usize = 10_000;
/// Most numbers one key's value may hold (a colour has four).
pub const MAX_NUMBERS: usize = 16;
/// Longest text value, in bytes.
pub const MAX_TEXT: usize = 1024;
/// Keys closer in time than this are at the same moment.
pub const SAME_TIME: f32 = 0.0005;

/// How a key's value changes towards the next key's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Ease {
    /// Holds the value, then jumps to the next at its moment.
    Step,
    /// Changes at an even rate.
    Linear,
    /// Starts and ends gently (smooth step).
    #[default]
    Smooth,
}

impl Ease {
    pub const ALL: [Ease; 3] = [Ease::Step, Ease::Linear, Ease::Smooth];

    /// How far along (0 to 1) the value is at `t` (0 to 1) of the way to the next key.
    pub fn progress(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Step => 0.0,
            Ease::Linear => t,
            Ease::Smooth => t * t * (3.0 - 2.0 * t),
        }
    }

    /// English label; the translation key.
    pub fn label(self) -> &'static str {
        match self {
            Ease::Step => "Step",
            Ease::Linear => "Linear",
            Ease::Smooth => "Smooth",
        }
    }
}

/// What a track animates.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    /// A component's field, such as `transform` `position` or `pointLight` `color`. A field inside
    /// an object field is named with dots (`a.b`).
    Field { component: String, field: String },
    /// A property of the object's script whose module is `script` (its first script with it).
    Property { script: String, property: String },
}

impl Target {
    pub fn field(component: &str, field: &str) -> Self {
        Self::Field { component: component.to_owned(), field: field.to_owned() }
    }

    pub fn property(script: &str, property: &str) -> Self {
        Self::Property { script: script.to_owned(), property: property.to_owned() }
    }

    /// Short text for messages: `transform.position`, `scripts/mover.wasm#speed`.
    pub fn describe(&self) -> String {
        match self {
            Target::Field { component, field } => format!("{component}.{field}"),
            Target::Property { script, property } => format!("{script}#{property}"),
        }
    }

    fn check(&self) -> Result<(), String> {
        let (a, b) = match self {
            Target::Field { component, field } => (component, field),
            Target::Property { script, property } => (script, property),
        };
        if a.is_empty() || b.is_empty() || a.len() > 256 || b.len() > 128 || a.contains('\0') || b.contains('\0') {
            return Err("a track names a component and field (or a script and property) of 1 to 256 bytes, without NUL".to_owned());
        }
        if matches!(self, Target::Field { component, .. } if component == "name" || component == "prefab") {
            return Err("an object's name and prefab link are not animated".to_owned());
        }
        Ok(())
    }
}

/// One moment of a track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    /// Seconds from the timeline's start.
    pub time: f32,
    /// The value at that moment: a number, a list of numbers, true/false or text.
    pub value: Value,
    /// How it eases to the next key.
    #[serde(default)]
    pub ease: Ease,
}

/// One animated value of a part, with its keys in time order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub target: Target,
    #[serde(default)]
    pub keys: Vec<Key>,
}

impl Track {
    /// The value at `time`: the first key's before it, the last key's after the last. `None`
    /// without keys.
    pub fn value_at(&self, time: f32) -> Option<Value> {
        let after = self.keys.partition_point(|k| k.time <= time);
        if after == 0 {
            return self.keys.first().map(|k| k.value.clone());
        }
        let key = &self.keys[after - 1];
        let Some(next) = self.keys.get(after) else { return Some(key.value.clone()) };
        let span = next.time - key.time;
        let t = if span > 0.0 { (time - key.time) / span } else { 1.0 };
        Some(blend(&key.value, &next.value, key.ease.progress(t)))
    }

    /// The index of the key at `time` (within [`SAME_TIME`]).
    pub fn key_at(&self, time: f32) -> Option<usize> {
        self.keys.iter().position(|k| (k.time - time).abs() <= SAME_TIME)
    }

    /// Puts a key in time order; one already at its moment is replaced. Returns its index.
    pub fn place(&mut self, key: Key) -> usize {
        if let Some(at) = self.key_at(key.time) {
            self.keys[at] = key;
            return at;
        }
        let at = self.keys.partition_point(|k| k.time < key.time);
        self.keys.insert(at, key);
        at
    }
}

/// A part of the prefab that the timeline animates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// The object's id in the prefab file.
    pub id: StableId,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

impl Part {
    pub fn track(&self, target: &Target) -> Option<&Track> {
        self.tracks.iter().find(|t| &t.target == target)
    }

    pub fn track_mut(&mut self, target: &Target) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| &t.target == target)
    }
}

/// A keyframe animation made for one prefab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Timeline {
    pub format: String,
    pub version: u32,
    /// The prefab it animates, as prefab links name it (relative to the project folder).
    pub prefab: String,
    /// Seconds, more than 0.
    pub length: f32,
    /// Starts over at its end; otherwise it stays at its last moment.
    #[serde(default)]
    pub looping: bool,
    #[serde(default)]
    pub parts: Vec<Part>,
}

/// One track's value at a moment.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample<'a> {
    pub part: StableId,
    pub target: &'a Target,
    pub value: Value,
}

impl Timeline {
    /// An empty timeline for `prefab`, `length` seconds long.
    pub fn new(prefab: &str, length: f32) -> Self {
        Self { format: FORMAT.to_owned(), version: VERSION, prefab: prefab.to_owned(), length, looping: false, parts: Vec::new() }
    }

    /// Reads a timeline file's text.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let timeline: Self = serde_json::from_str(text).map_err(|e| format!("not a timeline: {e}"))?;
        if timeline.format != FORMAT {
            return Err(format!("not a timeline (format \"{}\")", timeline.format));
        }
        if !(1..=VERSION).contains(&timeline.version) {
            return Err(format!("unsupported timeline version {}", timeline.version));
        }
        timeline.validate()?;
        Ok(timeline)
    }

    /// The file's text.
    pub fn to_json(&self) -> String {
        // Plain data: writing it does not fail.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Checks limits, finite times inside the length, keys in time order and values of a kind a
    /// track can hold.
    pub fn validate(&self) -> Result<(), String> {
        check_length(self.length)?;
        if self.prefab.is_empty() || self.prefab.len() > 1024 || self.prefab.contains('\0') {
            return Err("a timeline names its prefab, in 1 to 1024 bytes".to_owned());
        }
        if self.parts.len() > MAX_PARTS {
            return Err(format!("a timeline animates at most {MAX_PARTS} parts"));
        }
        for (i, part) in self.parts.iter().enumerate() {
            if self.parts[..i].iter().any(|p| p.id == part.id) {
                return Err(format!("part {} appears twice", part.id));
            }
            if part.tracks.len() > MAX_TRACKS {
                return Err(format!("a part has at most {MAX_TRACKS} tracks"));
            }
            for (j, track) in part.tracks.iter().enumerate() {
                track.target.check()?;
                if part.tracks[..j].iter().any(|t| t.target == track.target) {
                    return Err(format!("part {} has the track {} twice", part.id, track.target.describe()));
                }
                if track.keys.len() > MAX_KEYS {
                    return Err(format!("a track has at most {MAX_KEYS} keys"));
                }
                for (k, key) in track.keys.iter().enumerate() {
                    check_time(key.time, self.length)?;
                    check_value(&key.value)?;
                    if k > 0 && key.time - track.keys[k - 1].time <= SAME_TIME {
                        return Err(format!("the keys of {} are not in time order, or two share a moment", track.target.describe()));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn part(&self, id: StableId) -> Option<&Part> {
        self.parts.iter().find(|p| p.id == id)
    }

    pub fn part_mut(&mut self, id: StableId) -> Option<&mut Part> {
        self.parts.iter_mut().find(|p| p.id == id)
    }

    /// The moment of the timeline a clock `seconds` after it started shows: it wraps when the
    /// timeline loops and stops at the end when it plays once.
    pub fn local_time(&self, seconds: f64) -> f32 {
        let length = f64::from(self.length.max(SAME_TIME));
        let t = if self.looping { seconds.rem_euclid(length) } else { seconds.clamp(0.0, length) };
        t as f32
    }

    /// Whether a timeline that plays once is over `seconds` after it started.
    pub fn finished(&self, seconds: f64) -> bool {
        !self.looping && seconds >= f64::from(self.length)
    }

    /// Every track's value at `time` (seconds of the timeline), tracks without keys left out.
    pub fn sample(&self, time: f32) -> Vec<Sample<'_>> {
        self.parts
            .iter()
            .flat_map(|part| part.tracks.iter().filter_map(move |track| Some(Sample { part: part.id, target: &track.target, value: track.value_at(time)? })))
            .collect()
    }
}

/// Checks a timeline length.
pub fn check_length(length: f32) -> Result<(), String> {
    if length.is_finite() && length > SAME_TIME && length <= MAX_LENGTH {
        Ok(())
    } else {
        Err(format!("a timeline's length is more than 0 and at most {MAX_LENGTH} seconds"))
    }
}

/// Checks a key's time against the timeline's length.
pub fn check_time(time: f32, length: f32) -> Result<(), String> {
    if time.is_finite() && (0.0..=length + SAME_TIME).contains(&time) {
        Ok(())
    } else {
        Err(format!("a key's time is between 0 and the timeline's length ({length} seconds)"))
    }
}

/// Checks that a key's value is one a track can hold: a finite number, up to [`MAX_NUMBERS`]
/// finite numbers, true/false or text.
pub fn check_value(value: &Value) -> Result<(), String> {
    let ok = match value {
        Value::Number(n) => n.as_f64().is_some_and(f64::is_finite),
        Value::Bool(_) => true,
        Value::String(s) => s.len() <= MAX_TEXT && !s.contains('\0'),
        Value::Array(items) => !items.is_empty() && items.len() <= MAX_NUMBERS && items.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)),
        Value::Null | Value::Object(_) => false,
    };
    if ok { Ok(()) } else { Err(format!("a key's value is a finite number, 1 to {MAX_NUMBERS} finite numbers, true/false or text")) }
}

/// The value `amount` (0 to 1) of the way from `a` to `b`. Numbers and lists of numbers of the same
/// length blend; anything else holds `a` until `b`'s moment.
pub fn blend(a: &Value, b: &Value, amount: f32) -> Value {
    let amount = f64::from(amount);
    let mix = |x: f64, y: f64| x + (y - x) * amount;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => number(mix(x, y)),
            _ => a.clone(),
        },
        (Value::Array(xs), Value::Array(ys)) if xs.len() == ys.len() => {
            let mixed: Option<Vec<Value>> = xs.iter().zip(ys).map(|(x, y)| Some(number(mix(x.as_f64()?, y.as_f64()?)))).collect();
            mixed.map_or_else(|| a.clone(), Value::Array)
        }
        _ if amount >= 1.0 => b.clone(),
        _ => a.clone(),
    }
}

fn number(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

/// Which object of a scene stands for each part of a timeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Binding {
    pub objects: BTreeMap<StableId, StableId>,
}

impl Binding {
    /// Parts whose ids are the scene's own (the prefab open on its own).
    pub fn same_ids(scene: &Scene, timeline: &Timeline) -> Self {
        Self { objects: timeline.parts.iter().filter(|p| scene.contains(p.id)).map(|p| (p.id, p.id)).collect() }
    }

    /// Parts of a placed copy whose root is `root`: the prefab file lists its objects in hierarchy
    /// order (`file_ids`), and the copy holds them in the same order.
    pub fn by_order(scene: &Scene, root: StableId, file_ids: &[StableId], timeline: &Timeline) -> Self {
        let copy: Vec<StableId> = scene.export(&[root]).map(|objects| objects.into_iter().map(|o| o.id).collect()).unwrap_or_default();
        let objects = timeline
            .parts
            .iter()
            .filter_map(|p| {
                let at = file_ids.iter().position(|id| *id == p.id)?;
                Some((p.id, *copy.get(at)?))
            })
            .collect();
        Self { objects }
    }

    pub fn object(&self, part: StableId) -> Option<StableId> {
        self.objects.get(&part).copied()
    }

    /// The part `object` stands for.
    pub fn part_of(&self, object: StableId) -> Option<StableId> {
        self.objects.iter().find(|(_, o)| **o == object).map(|(p, _)| *p)
    }
}

/// The current value of `target` on `object`, as a track holds it.
pub fn read(scene: &Scene, object: StableId, target: &Target) -> Option<Value> {
    match target {
        Target::Field { component, field } => {
            let value = scene.component(object, component).ok()?;
            field_of(&value, field).cloned()
        }
        Target::Property { script, property } => {
            let scripts = scene.get::<Scripts>(object)?;
            let entry = scripts.0.iter().find(|s| &s.module == script)?;
            serde_json::to_value(entry.properties.get(property)?).ok()
        }
    }
}

/// The field at a dotted path inside a component's value.
pub fn field_of<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |v, name| v.get(name))
}

fn field_of_mut<'a>(value: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    path.split('.').try_fold(value, |v, name| v.get_mut(name))
}

/// Writes `samples` to the objects `binding` names, each component once. Parts with no object,
/// fields the object lacks and values its component refuses are skipped; what went wrong is
/// returned, once per track.
pub fn apply(scene: &mut Scene, binding: &Binding, samples: &[Sample<'_>]) -> Vec<String> {
    let mut problems = Vec::new();
    // Each object's changed components, written once.
    let mut changed: BTreeMap<(StableId, String), Value> = BTreeMap::new();
    let mut scripts: BTreeMap<StableId, Scripts> = BTreeMap::new();
    for sample in samples {
        let Some(object) = binding.object(sample.part) else { continue };
        match sample.target {
            Target::Field { component, field } => {
                let key = (object, component.clone());
                if !changed.contains_key(&key) {
                    match scene.component(object, component) {
                        Ok(value) => {
                            changed.insert(key.clone(), value);
                        }
                        Err(_) => {
                            problems.push(format!("{} has no {component} component", scene.name(object).unwrap_or("Object")));
                            continue;
                        }
                    }
                }
                let Some(slot) = changed.get_mut(&key).and_then(|v| field_of_mut(v, field)) else {
                    problems.push(format!("the {component} component has no field {field}"));
                    continue;
                };
                *slot = fit(slot, &sample.value);
            }
            Target::Property { script, property } => {
                if let std::collections::btree_map::Entry::Vacant(slot) = scripts.entry(object) {
                    let Some(current) = scene.get::<Scripts>(object).cloned() else {
                        problems.push(format!("{} has no scripts", scene.name(object).unwrap_or("Object")));
                        continue;
                    };
                    slot.insert(current);
                }
                let Some(entry) = scripts.get_mut(&object).and_then(|s| s.0.iter_mut().find(|s| &s.module == script)) else {
                    problems.push(format!("{} has no script {script}", scene.name(object).unwrap_or("Object")));
                    continue;
                };
                match serde_json::from_value::<PropertyValue>(sample.value.clone()) {
                    Ok(value) => {
                        entry.properties.insert(property.clone(), value);
                    }
                    Err(_) => problems.push(format!("{property} of {script} cannot hold {}", sample.value)),
                }
            }
        }
    }
    for ((object, component), value) in changed {
        if let Err(e) = scene.set_component(object, &component, value) {
            problems.push(format!("{component} of {}: {e}", scene.name(object).unwrap_or("Object")));
        }
    }
    for (object, value) in scripts {
        if let Err(e) = scene.set(object, value) {
            problems.push(format!("scripts of {}: {e}", scene.name(object).unwrap_or("Object")));
        }
    }
    problems
}

/// `value` in the shape of the field it goes into: a colour key of three numbers keeps the
/// field's alpha, a whole number stays whole.
fn fit(current: &Value, value: &Value) -> Value {
    match (current, value) {
        (Value::Array(now), Value::Array(new)) if new.len() < now.len() => {
            let mut out = new.clone();
            out.extend(now[new.len()..].iter().cloned());
            Value::Array(out)
        }
        (Value::Number(now), Value::Number(new)) if now.is_u64() || now.is_i64() => {
            new.as_f64().map(f64::round).map_or_else(|| value.clone(), |v| if v >= 0.0 && now.is_u64() { Value::from(v as u64) } else { Value::from(v as i64) })
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::components::{ScriptRef, Transform};

    fn track(target: Target, keys: &[(f32, Value, Ease)]) -> Track {
        Track { target, keys: keys.iter().map(|(time, value, ease)| Key { time: *time, value: value.clone(), ease: *ease }).collect() }
    }

    fn at(track: &Track, time: f32) -> f64 {
        track.value_at(time).and_then(|v| v.as_f64()).unwrap_or(f64::NAN)
    }

    #[test]
    fn each_ease_reaches_the_next_key_its_own_way() {
        let keys = |ease| track(Target::field("pointLight", "intensity"), &[(0.0, json!(0.0), ease), (2.0, json!(10.0), Ease::Linear)]);
        let (step, linear, smooth) = (keys(Ease::Step), keys(Ease::Linear), keys(Ease::Smooth));
        assert_eq!(at(&step, 1.9), 0.0, "a step holds its value");
        assert_eq!(at(&step, 2.0), 10.0, "and jumps at the next key");
        assert!((at(&linear, 1.0) - 5.0).abs() < 1e-6, "linear is halfway at half the time");
        assert!(at(&smooth, 0.5) < at(&linear, 0.5), "smooth starts more gently than linear");
        assert!(at(&smooth, 1.5) > at(&linear, 1.5), "and ends more gently");
        assert!((at(&smooth, 1.0) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn values_hold_before_the_first_key_and_after_the_last() {
        let t = track(Target::field("transform", "position"), &[(1.0, json!([1, 2, 3]), Ease::Linear), (2.0, json!([3, 2, 1]), Ease::Linear)]);
        assert_eq!(t.value_at(0.0), Some(json!([1, 2, 3])));
        assert_eq!(t.value_at(5.0), Some(json!([3, 2, 1])));
        assert_eq!(t.value_at(1.5), Some(json!([2.0, 2.0, 2.0])), "lists of numbers blend item by item");
        let text = track(Target::property("a.wasm", "say"), &[(0.0, json!("hi"), Ease::Linear), (1.0, json!("bye"), Ease::Linear)]);
        assert_eq!(text.value_at(0.9), Some(json!("hi")), "what cannot blend holds until the next key");
    }

    #[test]
    fn a_looping_timeline_wraps_and_one_that_plays_once_stops_at_its_end() {
        let mut timeline = Timeline::new("door.bprefab", 2.0);
        assert_eq!(timeline.local_time(5.0), 2.0);
        assert!(timeline.finished(2.5));
        timeline.looping = true;
        assert!((timeline.local_time(5.0) - 1.0).abs() < 1e-6);
        assert!(!timeline.finished(100.0));
    }

    #[test]
    fn placing_a_key_keeps_time_order_and_replaces_one_at_the_same_moment() {
        let mut t = track(Target::field("transform", "scale"), &[]);
        for time in [2.0, 0.5, 1.0] {
            t.place(Key { time, value: json!(time), ease: Ease::Linear });
        }
        t.place(Key { time: 1.0002, value: json!(9), ease: Ease::Step });
        let times: Vec<f32> = t.keys.iter().map(|k| k.time).collect();
        assert_eq!(times, [0.5, 1.0002, 2.0]);
        assert_eq!(t.keys[1].value, json!(9));
    }

    #[test]
    fn broken_timelines_are_refused() {
        let good = || {
            let mut timeline = Timeline::new("door.bprefab", 2.0);
            timeline.parts.push(Part {
                id: StableId::from_u128(1),
                tracks: vec![track(Target::field("transform", "position"), &[(0.0, json!([0, 0, 0]), Ease::Smooth)])],
            });
            timeline
        };
        assert!(Timeline::from_json(&good().to_json()).is_ok_and(|t| t == good()), "a timeline reads back as written");
        let mut late = good();
        late.parts[0].tracks[0].keys[0].time = 3.0;
        let mut unordered = good();
        unordered.parts[0].tracks[0].keys.push(Key { time: 0.0, value: json!([1, 1, 1]), ease: Ease::Step });
        let mut object = good();
        object.parts[0].tracks[0].keys[0].value = json!({"x": 1});
        let mut twice = good();
        twice.parts.push(twice.parts[0].clone());
        let mut name = good();
        name.parts[0].tracks[0].target = Target::field("name", "x");
        let mut long = good();
        long.length = f32::INFINITY;
        for broken in [late, unordered, object, twice, name, long] {
            assert!(broken.validate().is_err(), "{broken:?}");
        }
        assert!(Timeline::from_json(&good().to_json().replace(FORMAT, "bayleaf.scene")).is_err());
    }

    #[test]
    fn applying_samples_writes_each_part_on_its_bound_object() {
        let mut scene = Scene::new();
        let root = scene.create_object(None, "Lamp").unwrap();
        let bulb = scene.create_object(Some(root), "Bulb").unwrap();
        scene.add_component(bulb, "pointLight").unwrap();
        scene.set(bulb, Scripts(vec![ScriptRef { module: "flicker.wasm".to_owned(), properties: BTreeMap::new() }])).unwrap();
        // The prefab file's ids differ from the copy's: the copy is found by hierarchy order.
        let (file_root, file_bulb) = (StableId::from_u128(10), StableId::from_u128(11));
        let mut timeline = Timeline::new("lamp.bprefab", 1.0);
        timeline.parts.push(Part { id: file_root, tracks: vec![track(Target::field("transform", "position"), &[(0.0, json!([0, 5, 0]), Ease::Step)])] });
        timeline.parts.push(Part {
            id: file_bulb,
            tracks: vec![
                track(Target::field("pointLight", "color"), &[(0.0, json!([1, 0, 0]), Ease::Step)]),
                track(Target::property("flicker.wasm", "rate"), &[(0.0, json!(3.0), Ease::Step)]),
                track(Target::field("mesh", "color"), &[(0.0, json!([1, 0, 0]), Ease::Step)]),
            ],
        });
        let binding = Binding::by_order(&scene, root, &[file_root, file_bulb], &timeline);
        let problems = apply(&mut scene, &binding, &timeline.sample(0.5));
        assert_eq!(scene.get::<Transform>(root).map(|t| t.position), Some([0.0, 5.0, 0.0]));
        assert_eq!(read(&scene, bulb, &Target::field("pointLight", "color")), Some(json!([1.0, 0.0, 0.0])));
        assert_eq!(read(&scene, bulb, &Target::property("flicker.wasm", "rate")), Some(json!(3.0)));
        assert_eq!(problems.len(), 1, "the missing mesh is reported and the rest still applies: {problems:?}");
    }
}
