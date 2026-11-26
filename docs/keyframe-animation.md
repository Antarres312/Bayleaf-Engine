# Keyframe animation

A keyframe animation (`.btimeline`, `rust/crates/scene/src/timeline.rs`) is made for one prefab. It names
the prefab, has a length in seconds, plays once or loops, and lists the parts that take part: objects
of the prefab's hierarchy, by their ids in the prefab file. Each part has tracks, one value each:

- `{"component": "transform", "field": "position"}` (or `rotation`, `scale`), or any other
  component's field (`{"component": "pointLight", "field": "color"}`; a field inside an object field
  is named with dots);
- `{"script": "scripts/flicker.wasm", "property": "rate"}`: a property of the part's script with that
  module.

A track's keys are in time order. Each holds a value (a number, up to 16 numbers, true/false or text)
and says how it eases to the next key: `step` holds the value until the next key, `linear` goes evenly,
`smooth` (the default) starts and ends gently. Numbers and lists of numbers of the same length blend;
anything else holds until the next key. Before the first key a track has the first key's value, after
the last the last key's.

```json
{
  "format": "bayleaf.timeline",
  "version": 1,
  "prefab": "props/lamp.bprefab",
  "length": 2.0,
  "looping": true,
  "parts": [
    { "id": "<32 hex digits>", "tracks": [
      { "target": { "component": "transform", "field": "position" },
        "keys": [ { "time": 0.0, "value": [0, 0, 0], "ease": "linear" },
                  { "time": 2.0, "value": [0, 1, 0], "ease": "smooth" } ] } ] }
  ]
}
```

## Which objects are the parts

Opening a prefab on its own (`prefab.open`) keeps its file's object ids, so there the parts are the
objects themselves. A placed copy holds the file's objects in the same hierarchy order, so a part is
found at its place in that order (`Binding::by_order`). Objects inside a copy of another prefab nested
in this one get new ids each time that copy updates from its file, so they cannot take part.

## Playing a timeline from other code

The interface is three calls, so animation states can drive timelines like any other clip:

- `Timeline::local_time(seconds)`: the moment a clock shows, wrapped when the timeline loops and
  held at the end when it plays once (`Timeline::finished` says when that is);
- `Timeline::sample(time)`: every track's value at that moment, as `Sample { part, target, value }`;
- `timeline::apply(scene, &binding, &samples)`: writes them to the objects a `Binding` names, each
  component once, and returns what could not be set.

Blending two timelines is blending their samples (`timeline::blend`) before `apply`.

## Editing

Every edit is a `timeline.*` command and one undo step (`engine.commands` lists their params):
`timeline.new`, `open`, `save`, `close`, `get`, `set` (length, looping), `part.add`, `part.remove`,
`track.add`, `track.remove`, `key.set`, `key.move`, `key.remove`, `key.copy`, `key.paste`.

`timeline.seek` moves the playhead and shows the prefab's pose there, without play mode: the pose is
never an undo step and never dirties the prefab or the scene, and the values it replaced come back
before any other command runs (`timeline.pose.end` puts them back at once). With `timeline.record`
on, `scene.component.set` and `scene.script.setProperty` on a posed part become `timeline.key.record`:
the changed values are keyed at the playhead instead of changing the prefab. That is how the scene
view's handles and the Inspector place keys while recording.

The editor's Keyframes panel shows the tracks under a time ruler; its keyboard use is described at the
top of `rust/crates/ui-egui/src/timeline_view.rs`.

## In play mode

A Keyframe Animation component (`keyframeAnimation`: `animation`, `playOnStart`, `speed`) on the
prefab copy's root plays the file on that copy. `timeline.play {id}` and `timeline.stop {id}`, the
`timeline_play` and `timeline_stop` host functions (Rust: `Object::play_keyframes`,
`Object::stop_keyframes`) and the Play and Stop Keyframe Animation nodes start it from the beginning
and stop it where it is.
