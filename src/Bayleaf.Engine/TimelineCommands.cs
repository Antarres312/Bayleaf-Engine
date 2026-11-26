using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using TimelineModel = Bayleaf.Scene.Timeline;

namespace Bayleaf.Engine;

/// <summary>The timeline open in the editor.</summary>
internal sealed class TimelineDoc(TimelineModel timeline, string? path)
{
    public TimelineModel Timeline { get; set; } = timeline;

    /// <summary>File it was opened from or last saved to.</summary>
    public string? Path { get; set; } = path;

    /// <summary>True when it has changes that are not saved.</summary>
    public bool Dirty { get; set; }

    /// <summary>The moment shown, in seconds.</summary>
    public float Playhead { get; set; }

    /// <summary>Edits of posed parts become keys at the playhead.</summary>
    public bool Recording { get; set; }

    /// <summary>The values the pose on show replaced (object, component, value), put back when it ends; null while nothing is posed.</summary>
    public List<(StableId Object, string Component, JsonValue Value)>? Rest { get; set; }

    public bool IsPosed => Rest is not null;
}

/// <summary>
/// Keyframe animation commands (<c>timeline.*</c>): make, edit, save and preview <c>.btimeline</c> files.
/// One timeline is open at a time; its edits are undo steps in the same history as scene edits. The
/// playhead poses the prefab the timeline is made for without play mode: the prefab open on its own,
/// or else the first copy of it in the scene. The pose is shown, not kept: it is never an undo step,
/// never dirties the file, and the values it replaced come back before any other command runs. While
/// recording, edits of a posed part become keys at the playhead.
/// </summary>
internal static class TimelineCommands
{
    /// <summary>Why timeline edits are disabled with no timeline open.</summary>
    public const string NoTimeline = "open or create a timeline first (timeline.open, timeline.new)";

    /// <summary><c>format</c> of copied keys.</summary>
    public const string KeysFormat = "bayleaf.timeline.keys";

    /// <summary>Most keys one copy or paste may hold.</summary>
    private const int MaxCopied = 10_000;

    private static string? HasTimeline(Session session) => session.Timeline is null ? NoTimeline : null;

    private static CommandSpec Spec(string id, string label, string parameters, Mutates mutates, Func<Session, JsonValue, JsonValue> run) => new()
    {
        Id = id, Label = label, Params = parameters, Mutates = mutates, Run = run,
        Enabled = id is "timeline.new" or "timeline.open" ? Session.Always : HasTimeline,
    };

    public static IEnumerable<CommandSpec> All() =>
    [
        Spec("timeline.new", "New Keyframe Animation", "{prefab, length?, path?}: opens a new timeline for the .bprefab file (length in seconds, 2 if left out); with path it is written there at once", Mutates.No, New),
        Spec("timeline.open", "Open Keyframe Animation", "{path} (.btimeline)", Mutates.No, Open),
        Spec("timeline.save", "Save Keyframe Animation", "{path?} (needed for a timeline that has no file yet)", Mutates.No, Save),
        Spec("timeline.close", "Close Keyframe Animation", "{discard?}: closes the timeline with its undo steps; refuses unsaved changes unless discard", Mutates.No, Close),
        Spec("timeline.get", "Show Keyframe Animation", "none; returns {path, dirty, playhead, recording, posed, timeline, parts: [{id, name, object}]}; object is the scene object that stands for the part, when the prefab is open or placed", Mutates.No, Get),
        Spec("timeline.set", "Change Keyframe Animation", "{length?, looping?}: a shorter length drops the keys after it; returns {dropped}", Mutates.Timeline, Set),
        Spec("timeline.part.add", "Add Animated Part", "{part}: an object of the prefab, by its id in the prefab file", Mutates.Timeline, PartAdd),
        Spec("timeline.part.remove", "Remove Animated Part", "{part}: with its tracks", Mutates.Timeline, PartRemove),
        Spec("timeline.track.add", "Add Track", "{part, track}: track is {component, field} (transform position, rotation or scale; a field of another component, dotted inside an object field) or {script, property} (a property of the part's script with that module); the part is added too", Mutates.Timeline, TrackAdd),
        Spec("timeline.track.remove", "Remove Track", "{part, track}: with its keys", Mutates.Timeline, TrackRemove),
        Spec("timeline.key.set", "Set Key", "{part, track, time?, value?, ease?}: places a key (at the playhead when time is left out) or changes the one there; value left out keeps the key's or reads the part's value now; ease \"step\", \"linear\" or \"smooth\" (the default) says how it goes to the next key; the track and part are added too", Mutates.Timeline, KeySet),
        Spec("timeline.key.move", "Move Keys", "{keys: [{part, track, time}], by}: moves the keys by seconds, inside the length; a key moved onto another's moment replaces it; returns {keys}, where they are now", Mutates.Timeline, KeyMove),
        Spec("timeline.key.remove", "Delete Keys", "{keys: [{part, track, time}]}", Mutates.Timeline, KeyRemove),
        Spec("timeline.key.copy", "Copy Keys", "{keys: [{part, track, time}]}: returns {text}, the keys as clipboard text", Mutates.No, KeyCopy),
        Spec("timeline.key.paste", "Paste Keys", "{text, time?, part?, track?}: adds copied keys, the earliest at time (else the playhead), on their own tracks, or all on part and track when given; returns {keys}", Mutates.Timeline, KeyPaste),
        Spec("timeline.key.record", "Record Key", "{id, component, value} or {id, index, name, value}: what scene.component.set and scene.script.setProperty take; keys the changed values of the posed part at the playhead (recording turns this on for those commands); returns {keys}", Mutates.Timeline, KeyRecord),
        Spec("timeline.seek", "Move Playhead", "{time}: shows the prefab's pose at that moment, without play mode or an undo step; any other command puts the values back first; returns {time, problems}", Mutates.No, Seek),
        Spec("timeline.pose.end", "Show Without Animation", "none: puts back the values the pose replaced", Mutates.No, PoseEnd),
        Spec("timeline.record", "Record Keys", "{on}: while on, edits of a posed part (scene.component.set, scene.script.setProperty) become keys at the playhead", Mutates.No, Record),
    ];

    private static TimelineDoc Doc(Session session) => session.Timeline ?? throw Stop.Failed(NoTimeline);

    // The prefab a timeline is made for, and the objects that stand for its parts.

    private static bool PrefabIsOpen(Session session, string prefab) => session.PrefabEdits.Count > 0 && session.PrefabEdits[^1].Source == prefab;

    /// <summary>The ids of the prefab's objects in hierarchy order: the scene's own while the prefab is open, else its file's.</summary>
    private static List<StableId> PrefabIds(Session session, string prefab)
    {
        if (PrefabIsOpen(session, prefab))
        {
            return session.Scene.Export(session.Scene.Children(null)).OrFail().Select(o => o.Id).ToList();
        }
        return PrefabCommands.ReadPrefab(session, prefab).Select(o => o.Id).ToList();
    }

    /// <summary>The objects that stand for the timeline's parts: the prefab's own while it is open, else the first copy of it in the scene.</summary>
    internal static Binding BindingOf(Session session, TimelineModel timeline)
    {
        if (PrefabIsOpen(session, timeline.Prefab))
        {
            return Binding.SameIds(session.Scene, timeline);
        }
        var copy = FirstCopy(session, timeline.Prefab) ?? throw Stop.Failed($"open {timeline.Prefab} (prefab.open) or place a copy of it in the scene to see the timeline's pose");
        return Binding.ByOrder(session.Scene, copy, PrefabIds(session, timeline.Prefab), timeline);
    }

    private static Binding? TryBinding(Session session, TimelineModel timeline)
    {
        try
        {
            return BindingOf(session, timeline);
        }
        catch (CommandException)
        {
            return null;
        }
    }

    /// <summary>The component a track writes.</summary>
    private static string ComponentOf(Target target) => target is Target.Field f ? f.Component : "scripts";

    // The pose at the playhead.

    /// <summary>Puts back the values the pose replaced.</summary>
    internal static void EndPose(Session session)
    {
        if (session.Timeline?.Rest is not { } rest)
        {
            return;
        }
        session.Timeline.Rest = null;
        foreach (var (obj, component, value) in rest)
        {
            if (session.Scene.Contains(obj))
            {
                session.Scene.SetComponent(obj, component, value);
            }
        }
    }

    /// <summary>Shows the pose at the playhead, from the values the scene holds without it. Returns what could not be shown.</summary>
    private static IReadOnlyList<string> Pose(Session session)
    {
        EndPose(session);
        var binding = BindingOf(session, Doc(session).Timeline);
        var doc = Doc(session);
        var samples = doc.Timeline.SampleAt(doc.Playhead);
        var rest = new List<(StableId, string, JsonValue)>();
        foreach (var sample in samples)
        {
            if (binding.ObjectOf(sample.Part) is not { } obj)
            {
                continue;
            }
            var component = ComponentOf(sample.Target);
            if (!rest.Exists(r => r.Item1 == obj && r.Item2 == component) && session.Scene.Component(obj, component) is { IsOk: true } value)
            {
                rest.Add((obj, component, value.Value));
            }
        }
        var problems = TimelineModel.Apply(session.Scene, binding, samples);
        doc.Rest = rest;
        return problems;
    }

    /// <summary>Before any command but the timeline's own (and undo or redo of a timeline step), the pose ends.</summary>
    internal static void BeforeCommand(Session session, string id)
    {
        if (id.StartsWith("timeline.", StringComparison.Ordinal))
        {
            return;
        }
        var step = id switch
        {
            "edit.undo" => session.Undo.Count > 0 ? session.Undo[^1] : null,
            "edit.redo" => session.Redo.Count > 0 ? session.Redo[^1] : null,
            _ => null,
        };
        if (step?.Before.Kind == Session.SnapshotKind.Timeline)
        {
            return;
        }
        EndPose(session);
    }

    /// <summary>After the timeline changed: the pose on show follows it.</summary>
    internal static void AfterChange(Session session)
    {
        if (session.Timeline is { IsPosed: true })
        {
            try
            {
                Pose(session);
            }
            catch (CommandException)
            {
                // The pose cannot be shown now; it shows again with the next seek.
            }
        }
    }

    /// <summary>While recording, the commands that change a posed part's values become <c>timeline.key.record</c>.</summary>
    internal static bool Redirect(Session session, string id, JsonValue parameters)
    {
        if (id is not ("scene.component.set" or "scene.script.setProperty") || session.IsPlaying)
        {
            return false;
        }
        if (session.Timeline is not { Recording: true } doc)
        {
            return false;
        }
        if (id == "scene.component.set" && parameters.Get("component")?.AsString() is "name" or "prefab")
        {
            return false;
        }
        if (id == "scene.script.setProperty" && parameters.Get("value") is null or JsonNull)
        {
            return false;
        }
        if (parameters.Get("id")?.AsString() is not { } text || StableId.ParseHex(text) is not { IsOk: true } parsed)
        {
            return false;
        }
        var obj = parsed.Value;
        if (TryBinding(session, doc.Timeline) is not { } binding)
        {
            return false;
        }
        if (binding.PartOf(obj) is not null)
        {
            return true;
        }
        // A part not animated yet is one of the prefab's objects.
        try
        {
            return PartFor(session, binding, PrefabIds(session, doc.Timeline.Prefab), obj) is not null;
        }
        catch (CommandException)
        {
            return false;
        }
    }

    /// <summary>The part <paramref name="obj"/> stands for, among the prefab's ids, whether or not it is animated yet.</summary>
    private static StableId? PartFor(Session session, Binding binding, List<StableId> prefabIds, StableId obj)
    {
        if (binding.PartOf(obj) is { } part)
        {
            return part;
        }
        if (session.Timeline is not { } doc)
        {
            return null;
        }
        if (PrefabIsOpen(session, doc.Timeline.Prefab))
        {
            return prefabIds.Contains(obj) ? obj : null;
        }
        // A copy: the object's place in the copy's hierarchy order is the part's in the file.
        if (FirstCopy(session, doc.Timeline.Prefab) is not { } root || session.Scene.Export([root]) is not { IsOk: true } exported)
        {
            return null;
        }
        var at = exported.Value.Select(o => o.Id).ToList().IndexOf(obj);
        return at >= 0 && at < prefabIds.Count ? prefabIds[at] : null;
    }

    // Files.

    private static void RefuseUnsaved(Session session)
    {
        if (session.Timeline is { Dirty: true })
        {
            throw Stop.Failed("the open timeline has unsaved changes: save it (timeline.save) or close it with discard (timeline.close)");
        }
    }

    private static void CheckExtension(string path)
    {
        if (!string.Equals(System.IO.Path.GetExtension(path).TrimStart('.'), TimelineModel.Extension, StringComparison.Ordinal))
        {
            throw Stop.BadParams($"timeline files end in .{TimelineModel.Extension}");
        }
    }

    private static string Write(Session session, string path, TimelineModel timeline)
    {
        CheckExtension(path);
        path = session.Allowed(path, Access.Write);
        if (System.IO.Path.GetDirectoryName(path) is { Length: > 0 } parent)
        {
            Files.CreateDirectory(parent).MapError(e => $"cannot make {parent}: {e}").OrFail();
        }
        Files.Write(path, System.Text.Encoding.UTF8.GetBytes(timeline.ToJson())).MapError(e => $"cannot write {path}: {e}").OrFail();
        return path;
    }

    /// <summary>Puts <paramref name="doc"/> in place of the open timeline, whose undo steps go.</summary>
    private static void Replace(Session session, TimelineDoc doc)
    {
        EndPose(session);
        session.Forget(Session.SnapshotKind.Timeline);
        session.Timeline = doc;
    }

    private static JsonObject New(Session session, JsonValue p)
    {
        var f = Params.Of(p, "prefab", "length", "path");
        var prefab = f.Required("prefab", JsonRead.Str);
        float? length = f.NullableValue("length", JsonRead.F32);
        var path = f.Nullable("path", JsonRead.Str);
        RefuseUnsaved(session);
        if (!string.Equals(System.IO.Path.GetExtension(prefab).TrimStart('.'), PrefabCommands.Extension, StringComparison.Ordinal))
        {
            throw Stop.BadParams($"a timeline is made for a .{PrefabCommands.Extension} file");
        }
        var seconds = length ?? 2f;
        TimelineModel.CheckLength(seconds).OrBadParams();
        var source = PrefabCommands.LinkPath(session, System.IO.Path.Combine(session.ScriptDir, prefab));
        // The prefab must be there to animate (and readable under automation).
        PrefabIds(session, source);
        var timeline = new TimelineModel(source, seconds);
        var written = path is null ? null : Write(session, path, timeline);
        Replace(session, new TimelineDoc(timeline, written));
        return Get(session, JsonValue.Null);
    }

    private static JsonObject Open(Session session, JsonValue p)
    {
        var given = Params.Of(p, "path").Required("path", JsonRead.Str);
        CheckExtension(given);
        var path = session.Allowed(given, Access.Read);
        if (session.Timeline?.Path is { } open && SameFile(open, path))
        {
            return Get(session, JsonValue.Null);
        }
        RefuseUnsaved(session);
        var text = Files.ReadText(path);
        if (text.IsError)
        {
            throw Stop.Failed($"cannot read {path}: {text.Error}");
        }
        var timeline = TimelineModel.FromJson(text.Value);
        if (timeline.IsError)
        {
            throw Stop.Failed($"{path}: {timeline.Error}");
        }
        Replace(session, new TimelineDoc(timeline.Value, path));
        return Get(session, JsonValue.Null);
    }

    private static bool SameFile(string a, string b)
    {
        static string Canonical(string p) => File.Exists(p) ? System.IO.Path.GetFullPath(p) : p;
        return string.Equals(Canonical(a), Canonical(b), OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal);
    }

    private static JsonObject Save(Session session, JsonValue p)
    {
        var given = Params.Of(p, "path").Nullable("path", JsonRead.Str);
        var doc = Doc(session);
        var path = given ?? doc.Path ?? throw Stop.BadParams("this timeline has no file yet; give a path");
        var written = Write(session, path, doc.Timeline);
        doc.Path = written;
        doc.Dirty = false;
        return J.Obj(("path", J.S(written)));
    }

    private static JsonObject Close(Session session, JsonValue p)
    {
        var discard = Params.Of(p, "discard").Optional("discard", JsonRead.Bool, false);
        if (Doc(session).Dirty && !discard)
        {
            throw Stop.Failed("the timeline has unsaved changes: save it (timeline.save), or close it with discard");
        }
        EndPose(session);
        session.Forget(Session.SnapshotKind.Timeline);
        session.Timeline = null;
        return new JsonObject();
    }

    /// <summary>The prefab's objects' names by their file ids, from the scene where they stand for parts and else from the file.</summary>
    private static List<(StableId Id, string Name)> PartNames(Session session, TimelineModel timeline, Binding? binding)
    {
        List<ObjectDoc>? file = null;
        if (!PrefabIsOpen(session, timeline.Prefab))
        {
            try
            {
                file = PrefabCommands.ReadPrefab(session, timeline.Prefab);
            }
            catch (CommandException)
            {
                file = null;
            }
        }
        return timeline.Parts.Select(part =>
        {
            var fromScene = binding?.ObjectOf(part.Id) is { } obj ? session.Scene.NameOf(obj) : null;
            var fromFile = file?.Find(o => o.Id == part.Id) is { } o && o.Components.TryGetValue("name", out var name) ? name.AsString() : null;
            return (part.Id, fromScene ?? fromFile ?? "Missing part");
        }).ToList();
    }

    private static JsonObject Get(Session session, JsonValue p)
    {
        Params.None(p);
        var doc = Doc(session);
        var binding = TryBinding(session, doc.Timeline);
        var parts = new JsonArray(PartNames(session, doc.Timeline, binding).Select(part =>
            (JsonValue)J.Obj(("id", J.Id(part.Id)), ("name", J.S(part.Name)), ("object", J.Id(binding?.ObjectOf(part.Id))))));
        return J.Obj(
            ("path", J.S(doc.Path)),
            ("dirty", J.B(doc.Dirty)),
            ("playhead", JsonNumber.Of(doc.Playhead)),
            ("recording", J.B(doc.Recording)),
            ("posed", J.B(doc.IsPosed)),
            ("timeline", doc.Timeline.ToJsonObject()),
            ("parts", parts));
    }

    // Edits.

    private static JsonObject Set(Session session, JsonValue p)
    {
        var f = Params.Of(p, "length", "looping");
        float? length = f.NullableValue("length", JsonRead.F32);
        bool? looping = f.NullableValue("looping", JsonRead.Bool);
        if (length is { } l)
        {
            TimelineModel.CheckLength(l).OrBadParams();
        }
        var doc = Doc(session);
        var dropped = 0;
        if (length is { } newLength)
        {
            doc.Timeline.Length = newLength;
            foreach (var track in doc.Timeline.Parts.SelectMany(part => part.Tracks))
            {
                dropped += track.Keys.RemoveAll(k => !(k.Time <= newLength + TimelineModel.SameTime));
            }
            doc.Playhead = Math.Min(doc.Playhead, newLength);
        }
        if (looping is { } loops)
        {
            doc.Timeline.Looping = loops;
        }
        return J.Obj(("dropped", J.N(dropped)));
    }

    private static StableId PartParam(JsonValue p) => Params.Of(p, "part").Required("part", JsonRead.Id);

    /// <summary>The part of the open timeline, added when the prefab has that object.</summary>
    private static Part EnsurePart(Session session, StableId id)
    {
        var doc = Doc(session);
        if (doc.Timeline.FindPart(id) is { } found)
        {
            return found;
        }
        var prefab = doc.Timeline.Prefab;
        if (!PrefabIds(session, prefab).Contains(id))
        {
            throw Stop.BadParams($"{prefab} has no object {id}");
        }
        if (doc.Timeline.Parts.Count >= TimelineModel.MaxParts)
        {
            throw Stop.Failed($"a timeline animates at most {TimelineModel.MaxParts} parts");
        }
        var part = new Part(id, []);
        doc.Timeline.Parts.Add(part);
        return part;
    }

    private static JsonObject PartAdd(Session session, JsonValue p)
    {
        var id = PartParam(p);
        if (Doc(session).Timeline.FindPart(id) is not null)
        {
            throw Stop.Failed($"{id} already takes part");
        }
        EnsurePart(session, id);
        return new JsonObject();
    }

    private static JsonObject PartRemove(Session session, JsonValue p)
    {
        var id = PartParam(p);
        if (Doc(session).Timeline.Parts.RemoveAll(part => part.Id == id) == 0)
        {
            throw Stop.Failed($"{id} does not take part");
        }
        return new JsonObject();
    }

    private static (StableId Part, Target Track) TrackParams(JsonValue p)
    {
        var f = Params.Of(p, "part", "track");
        return (f.Required("part", JsonRead.Id), f.Required("track", Target.Read));
    }

    /// <summary>Checks a track's target the way a file is checked.</summary>
    private static void CheckTarget(Target target)
    {
        var probe = new TimelineModel("probe.bprefab", 1f);
        probe.Parts.Add(new Part(StableId.FromUInt128(0), [new Track(target, [])]));
        probe.Validate().OrBadParams();
    }

    private static Track EnsureTrack(Session session, StableId partId, Target target)
    {
        CheckTarget(target);
        var part = EnsurePart(session, partId);
        if (part.Track(target) is { } track)
        {
            return track;
        }
        if (part.Tracks.Count >= TimelineModel.MaxTracks)
        {
            throw Stop.Failed($"a part has at most {TimelineModel.MaxTracks} tracks");
        }
        var added = new Track(target, []);
        part.Tracks.Add(added);
        return added;
    }

    private static JsonObject TrackAdd(Session session, JsonValue p)
    {
        var (part, target) = TrackParams(p);
        if (Doc(session).Timeline.FindPart(part)?.Track(target) is not null)
        {
            throw Stop.Failed($"the part already has the track {target.Describe()}");
        }
        EnsureTrack(session, part, target);
        return new JsonObject();
    }

    private static JsonObject TrackRemove(Session session, JsonValue p)
    {
        var (partId, target) = TrackParams(p);
        var part = Doc(session).Timeline.FindPart(partId) ?? throw Stop.Failed($"{partId} does not take part");
        if (part.Tracks.RemoveAll(t => t.Target.Equals(target)) == 0)
        {
            throw Stop.Failed($"the part has no track {target.Describe()}");
        }
        return new JsonObject();
    }

    /// <summary>The object that stands for a part of the open timeline, whether or not it is animated yet.</summary>
    private static StableId? ObjectFor(Session session, StableId part)
    {
        if (session.Timeline is not { } doc)
        {
            return null;
        }
        if (PrefabIsOpen(session, doc.Timeline.Prefab))
        {
            return session.Scene.Contains(part) ? part : null;
        }
        try
        {
            var at = PrefabIds(session, doc.Timeline.Prefab).IndexOf(part);
            if (at < 0 || FirstCopy(session, doc.Timeline.Prefab) is not { } root || session.Scene.Export([root]) is not { IsOk: true } exported)
            {
                return null;
            }
            return at < exported.Value.Count ? exported.Value[at].Id : null;
        }
        catch (CommandException)
        {
            return null;
        }
    }

    /// <summary>The first copy of the prefab in the scene.</summary>
    private static StableId? FirstCopy(Session session, string prefab) =>
        session.Scene.WorldMatrices().Select(m => (StableId?)m.Id).FirstOrDefault(id => session.Scene.TryGet<Prefab>(id!.Value, out var link) && link.Source == prefab);

    private static JsonObject KeySet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "part", "track", "time", "value", "ease");
        var partId = f.Required("part", JsonRead.Id);
        var target = f.Required("track", Target.Read);
        float? given = f.NullableValue("time", JsonRead.F32);
        var value = f.Nullable("value", v => v.Canonical());
        Ease? ease = f.NullableValue("ease", Eases.Read);
        var doc = Doc(session);
        var time = given ?? doc.Playhead;
        TimelineModel.CheckTime(time, doc.Timeline.Length).OrBadParams();
        time = Math.Min(time, doc.Timeline.Length);
        var track = doc.Timeline.FindPart(partId)?.Track(target);
        var existing = track?.KeyAt(time) is { } at ? track.Keys[at] : null;
        value ??= existing?.Value
            ?? (ObjectFor(session, partId) is { } obj ? TimelineModel.ReadValue(session.Scene, obj, target) : null)
            ?? track?.ValueAt(time)
            ?? throw Stop.Failed($"give a value: the part is not in the scene to read its {target.Describe()} from");
        TimelineModel.CheckValue(value).OrBadParams();
        var keyEase = ease ?? existing?.Ease ?? Ease.Smooth;
        var placed = EnsureTrack(session, partId, target);
        if (placed.KeyAt(time) is null && placed.Keys.Count >= TimelineModel.MaxKeys)
        {
            throw Stop.Failed($"a track has at most {TimelineModel.MaxKeys} keys");
        }
        placed.Place(new Key(time, value, keyEase));
        return J.Obj(("time", JsonNumber.Of(time)));
    }

    /// <summary>One key, by its part, track and moment.</summary>
    private sealed record KeyRef(StableId Part, Target Track, float Time)
    {
        public static KeyRef Read(JsonValue value)
        {
            var f = JsonFields.Of(value, "struct KeyRef", true, "part", "track", "time");
            return new(f.Required("part", JsonRead.Id), f.Required("track", Target.Read), f.Required("time", JsonRead.F32));
        }

        public JsonObject Write() => J.Obj(("part", J.Id(Part)), ("track", Track.Write()), ("time", JsonNumber.Of(Time)));
    }

    private static JsonArray Refs(IEnumerable<KeyRef> refs) => new(refs.Select(r => (JsonValue)r.Write()));

    /// <summary>The keys the refs name, taken off their tracks, with where they were. Refuses a key that is not there.</summary>
    private static List<(KeyRef Ref, Key Key)> TakeKeys(TimelineModel timeline, IReadOnlyList<KeyRef> refs)
    {
        if (refs.Count > MaxCopied)
        {
            throw Stop.BadParams($"at most {MaxCopied} keys at once");
        }
        var taken = new List<(KeyRef, Key)>(refs.Count);
        foreach (var r in refs)
        {
            var missing = Stop.Failed($"there is no key of {r.Track.Describe()} at {Text.Number(r.Time)} seconds");
            var track = timeline.FindPart(r.Part)?.Track(r.Track) ?? throw missing;
            var at = track.KeyAt(r.Time) ?? throw missing;
            taken.Add((r, track.Keys[at]));
            track.Keys.RemoveAt(at);
        }
        return taken;
    }

    private static List<KeyRef> KeysParam(JsonFields f) => f.Required("keys", v => JsonRead.List(v, KeyRef.Read));

    private static JsonObject KeyMove(Session session, JsonValue p)
    {
        var f = Params.Of(p, "keys", "by");
        var refs = KeysParam(f);
        var by = f.Required("by", JsonRead.F32);
        if (!float.IsFinite(by))
        {
            throw Stop.BadParams("by is a finite number of seconds");
        }
        var timeline = Doc(session).Timeline;
        var length = timeline.Length;
        var moved = new List<KeyRef>();
        foreach (var (r, key) in TakeKeys(timeline, refs))
        {
            var time = Math.Clamp(key.Time + by, 0f, length);
            timeline.FindPart(r.Part)?.Track(r.Track)?.Place(key with { Time = time });
            moved.Add(r with { Time = time });
        }
        return J.Obj(("keys", Refs(moved)));
    }

    private static JsonObject KeyRemove(Session session, JsonValue p) =>
        J.Obj(("removed", J.N(TakeKeys(Doc(session).Timeline, KeysParam(Params.Of(p, "keys"))).Count)));

    private static JsonObject KeyCopy(Session session, JsonValue p)
    {
        var refs = KeysParam(Params.Of(p, "keys"));
        var copy = TimelineModel.FromJson(Doc(session).Timeline.ToJson()).OrFail();
        var keys = new JsonArray(TakeKeys(copy, refs).Select(t =>
        {
            var o = new JsonObject { { "part", J.Id(t.Ref.Part) }, { "track", t.Ref.Track.Write() } };
            foreach (var (k, v) in t.Key.Write().Entries)
            {
                o.Add(k, v);
            }
            return (JsonValue)o;
        }));
        var text = new JsonObject { { "format", new JsonString(KeysFormat) }, { "keys", keys } }.ToCompact();
        return J.Obj(("text", J.S(text)));
    }

    private static JsonObject KeyPaste(Session session, JsonValue p)
    {
        var f = Params.Of(p, "text", "time", "part", "track");
        var text = f.Required("text", JsonRead.Str);
        float? at = f.NullableValue("time", JsonRead.F32);
        var part = Params.OptionalId(f, "part");
        var track = f.Nullable("track", Target.Read);
        if (part is null != track is null)
        {
            throw Stop.BadParams("give part and track together, or neither");
        }
        var parsed = JsonReader.Parse(text);
        var copied = parsed.IsOk
            ? JsonRead.Try(() =>
            {
                var c = JsonFields.Of(parsed.Value, "struct Copied", true, "format", "keys");
                return (Format: c.Required("format", JsonRead.Str), Keys: c.Required("keys", v => JsonRead.List(v, k =>
                {
                    var kf = JsonFields.Of(k, "struct CopiedKey", true, "part", "track", "time", "value", "ease");
                    return (Part: kf.Required("part", JsonRead.Id), Track: kf.Required("track", Target.Read),
                        Key: new Key(kf.Required("time", JsonRead.F32), kf.Required("value", x => x.Canonical()), kf.Optional("ease", Eases.Read, Ease.Smooth)));
                })));
            })
            : Result.Fail(parsed.Error);
        if (copied.IsError)
        {
            throw Stop.BadParams($"not copied keys: {copied.Error}");
        }
        if (copied.Value.Format != KeysFormat || copied.Value.Keys.Count > MaxCopied)
        {
            throw Stop.BadParams("not copied keys");
        }
        var doc = Doc(session);
        var (length, playhead) = (doc.Timeline.Length, doc.Playhead);
        var start = at ?? playhead;
        TimelineModel.CheckTime(start, length).OrBadParams();
        var earliest = copied.Value.Keys.Select(k => k.Key.Time).DefaultIfEmpty(float.PositiveInfinity).Min();
        var pasted = new List<KeyRef>();
        foreach (var copy in copied.Value.Keys)
        {
            var (toPart, toTrack) = part is { } given && track is not null ? (given, track) : (copy.Part, copy.Track);
            TimelineModel.CheckValue(copy.Key.Value).OrBadParams();
            var time = Math.Clamp(copy.Key.Time - earliest + start, 0f, length);
            var placed = EnsureTrack(session, toPart, toTrack);
            if (placed.KeyAt(time) is null && placed.Keys.Count >= TimelineModel.MaxKeys)
            {
                throw Stop.Failed($"a track has at most {TimelineModel.MaxKeys} keys");
            }
            placed.Place(copy.Key with { Time = time });
            pasted.Add(new KeyRef(toPart, toTrack, time));
        }
        return J.Obj(("keys", Refs(pasted)));
    }

    /// <summary>The leaves of <paramref name="now"/> that differ from <paramref name="old"/>, by dotted path.</summary>
    private static void ChangedFields(JsonValue old, JsonValue now, string path, List<(string Path, JsonValue Value)> output)
    {
        if (old is JsonObject o && now is JsonObject n)
        {
            foreach (var (name, value) in ((JsonObject)n.Canonical()).Entries)
            {
                var inner = path.Length == 0 ? name : $"{path}.{name}";
                if (o.Find(name) is { } was)
                {
                    ChangedFields(was, value, inner, output);
                }
                else
                {
                    output.Add((inner, value));
                }
            }
        }
        else if (!SameValue(old, now))
        {
            output.Add((path, now));
        }
    }

    /// <summary>Equal values, with numbers compared by value (<c>1</c> and <c>1.0</c> are the same).</summary>
    private static bool SameValue(JsonValue a, JsonValue b) => (a, b) switch
    {
        (JsonNumber x, JsonNumber y) => x.ToDouble() == y.ToDouble(),
        (JsonArray xs, JsonArray ys) => xs.Count == ys.Count && xs.Zip(ys).All(pair => SameValue(pair.First, pair.Second)),
        _ => a.Equals(b),
    };

    private static JsonObject KeyRecord(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "component", "index", "name", "value");
        var id = f.Required("id", JsonRead.Id);
        var component = f.Nullable("component", JsonRead.Str);
        int? index = f.NullableValue("index", Params.Index);
        var name = f.Nullable("name", JsonRead.Str);
        var value = f.Required("value", v => v.Canonical());
        var doc = Doc(session);
        var binding = BindingOf(session, doc.Timeline);
        var ids = PrefabIds(session, doc.Timeline.Prefab);
        var part = PartFor(session, binding, ids, id) ?? throw Stop.Failed("this object is not a part of the timeline's prefab");
        // The changed values, keyed at the playhead.
        List<(Target Target, JsonValue Value)> changes;
        if (component is not null && index is null && name is null)
        {
            if (component is "name" or "prefab")
            {
                throw Stop.BadParams("an object's name and prefab link are not animated");
            }
            var now = session.Scene.Component(id, component).OrFail();
            if (now is not JsonObject || value is not JsonObject)
            {
                throw Stop.Failed($"the {component} component's fields cannot be keyed one by one");
            }
            var fields = new List<(string, JsonValue)>();
            ChangedFields(now, value, "", fields);
            changes = fields.Select(c => (Target.OfField(component, c.Item1), c.Item2)).ToList();
        }
        else if (component is null && index is { } at && name is not null)
        {
            var scripts = session.Scene.TryGet<Scripts>(id, out var s) ? s : throw Stop.Failed("the object has no scripts");
            var script = at < scripts.Items.Count ? scripts.Items[at] : throw Stop.BadParams($"the object has no script {at}");
            changes = [(Target.OfProperty(script.Module, name), value)];
        }
        else
        {
            throw Stop.BadParams("give {id, component, value} or {id, index, name, value}");
        }
        foreach (var (target, v) in changes)
        {
            CheckTarget(target);
            TimelineModel.CheckValue(v).MapError(e => $"{target.Describe()}: {e}").OrBadParams();
        }
        var time = doc.Playhead;
        var keys = new List<KeyRef>();
        foreach (var (target, v) in changes)
        {
            var track = EnsureTrack(session, part, target);
            var ease = track.KeyAt(time) is { } k ? track.Keys[k].Ease : Ease.Smooth;
            if (track.KeyAt(time) is null && track.Keys.Count >= TimelineModel.MaxKeys)
            {
                throw Stop.Failed($"a track has at most {TimelineModel.MaxKeys} keys");
            }
            track.Place(new Key(time, v, ease));
            keys.Add(new KeyRef(part, target, time));
        }
        // The recorded values show at once.
        doc.Rest ??= [];
        return J.Obj(("keys", Refs(keys)));
    }

    // The playhead and recording.

    private static JsonObject Seek(Session session, JsonValue p)
    {
        var time = Params.Of(p, "time").Required("time", JsonRead.F32);
        var doc = Doc(session);
        TimelineModel.CheckTime(time, doc.Timeline.Length).OrBadParams();
        doc.Playhead = Math.Min(time, doc.Timeline.Length);
        var problems = Pose(session);
        return J.Obj(("time", JsonNumber.Of(doc.Playhead)), ("problems", J.Strings(problems)));
    }

    private static JsonObject PoseEnd(Session session, JsonValue p)
    {
        Params.None(p);
        EndPose(session);
        return new JsonObject();
    }

    private static JsonObject Record(Session session, JsonValue p)
    {
        var on = Params.Of(p, "on").Required("on", JsonRead.Bool);
        if (on)
        {
            if (session.IsPlaying)
            {
                throw Stop.Failed("stop play mode first: play mode does not record keys");
            }
            // Recording shows the pose it keys.
            Pose(session);
        }
        Doc(session).Recording = on;
        return J.Obj(("recording", J.B(on)));
    }
}
