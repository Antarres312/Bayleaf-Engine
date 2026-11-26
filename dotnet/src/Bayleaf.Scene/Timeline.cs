using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;

namespace Bayleaf.Scene;

/// <summary>How a key's value changes towards the next key's.</summary>
public enum Ease
{
    /// <summary>Holds the value, then jumps to the next at its moment.</summary>
    Step,

    /// <summary>Changes at an even rate.</summary>
    Linear,

    /// <summary>Starts and ends gently (smooth step).</summary>
    Smooth,
}

public static class Eases
{
    public static readonly IReadOnlyList<Ease> All = [Ease.Step, Ease.Linear, Ease.Smooth];

    private static readonly string[] Names = ["step", "linear", "smooth"];

    /// <summary>How far along (0 to 1) the value is at <paramref name="t"/> (0 to 1) of the way to the next key.</summary>
    public static float Progress(this Ease ease, float t)
    {
        t = Math.Clamp(t, 0f, 1f);
        return ease switch
        {
            Ease.Step => 0f,
            Ease.Linear => t,
            _ => t * t * (3f - 2f * t),
        };
    }

    /// <summary>English label; the translation key.</summary>
    public static string Label(this Ease ease) => ease.ToString();

    internal static string Code(this Ease ease) => Names[(int)ease];

    internal static Ease Read(JsonValue value) => (Ease)JsonRead.Variant(value, Names);
}

/// <summary>What a track animates: a component's field, or a property of the object's script.</summary>
public abstract record Target
{
    private Target()
    {
    }

    /// <summary>A component's field, such as <c>transform</c> <c>position</c>; a field inside an object field is named with dots (<c>a.b</c>).</summary>
    public sealed record Field(string Component, string Name) : Target;

    /// <summary>A property of the object's script whose module is the script (its first script with it).</summary>
    public sealed record ScriptProperty(string Script, string Name) : Target;

    public static Target OfField(string component, string field) => new Field(component, field);

    public static Target OfProperty(string script, string property) => new ScriptProperty(script, property);

    /// <summary>Short text for messages: <c>transform.position</c>, <c>scripts/mover.wasm#speed</c>.</summary>
    public string Describe() => this switch
    {
        Field f => $"{f.Component}.{f.Name}",
        ScriptProperty p => $"{p.Script}#{p.Name}",
        _ => "",
    };

    internal Result<Unit, string> Check()
    {
        var (a, b) = this switch
        {
            Field f => (f.Component, f.Name),
            ScriptProperty p => (p.Script, p.Name),
            _ => ("", ""),
        };
        if (a.Length == 0 || b.Length == 0 || Text.Bytes(a) > 256 || Text.Bytes(b) > 128 || Text.HasNul(a) || Text.HasNul(b))
        {
            return Checks.Fail("a track names a component and field (or a script and property) of 1 to 256 bytes, without NUL");
        }
        return this is Field { Component: "name" or "prefab" } ? Checks.Fail("an object's name and prefab link are not animated") : Checks.Ok;
    }

    internal static Target Read(JsonValue value)
    {
        if (value is JsonObject o)
        {
            if (o.Find("component") is JsonString component && o.Find("field") is JsonString field)
            {
                return new Field(component.Value, field.Value);
            }
            if (o.Find("script") is JsonString script && o.Find("property") is JsonString property)
            {
                return new ScriptProperty(script.Value, property.Value);
            }
        }
        throw new JsonShapeException("data did not match any variant of untagged enum Target");
    }

    internal JsonValue Write() => this switch
    {
        Field f => new JsonObject { { "component", J.S(f.Component) }, { "field", J.S(f.Name) } },
        ScriptProperty p => new JsonObject { { "script", J.S(p.Script) }, { "property", J.S(p.Name) } },
        _ => JsonValue.Null,
    };

}

/// <summary>One moment of a track: the value at that moment and how it eases to the next key.</summary>
public sealed record Key(float Time, JsonValue Value, Ease Ease)
{
    internal static Key Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Key", true, "time", "value", "ease");
        return new(f.Required("time", JsonRead.F32), f.Required("value", v => v.Canonical()), f.Optional("ease", Eases.Read, Ease.Smooth));
    }

    internal JsonValue Write() => new JsonObject { { "time", J.F(Time) }, { "value", Value }, { "ease", J.S(Ease.Code()) } };
}

/// <summary>One animated value of a part, with its keys in time order.</summary>
public sealed class Track(Target target, List<Key> keys)
{
    public Target Target { get; } = target;

    public List<Key> Keys { get; } = keys;

    /// <summary>The value at <paramref name="time"/>: the first key's before it, the last key's after the last; null without keys.</summary>
    public JsonValue? ValueAt(float time)
    {
        var after = Keys.FindIndex(k => !(k.Time <= time));
        if (after < 0)
        {
            after = Keys.Count;
        }
        if (after == 0)
        {
            return Keys.Count > 0 ? Keys[0].Value : null;
        }
        var key = Keys[after - 1];
        if (after >= Keys.Count)
        {
            return key.Value;
        }
        var next = Keys[after];
        var span = next.Time - key.Time;
        var t = span > 0f ? (time - key.Time) / span : 1f;
        return Timeline.Blend(key.Value, next.Value, key.Ease.Progress(t));
    }

    /// <summary>The index of the key at <paramref name="time"/> (within <see cref="Timeline.SameTime"/>).</summary>
    public int? KeyAt(float time)
    {
        var at = Keys.FindIndex(k => Math.Abs(k.Time - time) <= Timeline.SameTime);
        return at >= 0 ? at : null;
    }

    /// <summary>Puts a key in time order; one already at its moment is replaced. Returns its index.</summary>
    public int Place(Key key)
    {
        if (KeyAt(key.Time) is { } existing)
        {
            Keys[existing] = key;
            return existing;
        }
        var at = Keys.FindIndex(k => !(k.Time < key.Time));
        at = at < 0 ? Keys.Count : at;
        Keys.Insert(at, key);
        return at;
    }

    internal static Track Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Track", true, "target", "keys");
        return new(f.Required("target", Target.Read), f.Optional("keys", v => JsonRead.List(v, Key.Read), []));
    }

    internal JsonValue Write() => new JsonObject { { "target", Target.Write() }, { "keys", new JsonArray(Keys.Select(k => k.Write())) } };
}

/// <summary>A part of the prefab that the timeline animates: the object's id in the prefab file, and its tracks.</summary>
public sealed class Part(StableId id, List<Track> tracks)
{
    public StableId Id { get; } = id;

    public List<Track> Tracks { get; } = tracks;

    public Track? Track(Target target) => Tracks.Find(t => t.Target.Equals(target));

    internal static Part Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Part", true, "id", "tracks");
        return new(f.Required("id", JsonRead.Id), f.Optional("tracks", v => JsonRead.List(v, Bayleaf.Scene.Track.Read), []));
    }

    internal JsonValue Write() => new JsonObject { { "id", J.S(Id.ToHex()) }, { "tracks", new JsonArray(Tracks.Select(t => t.Write())) } };
}

/// <summary>One track's value at a moment.</summary>
public sealed record Sample(StableId Part, Target Target, JsonValue Value);

/// <summary>
/// A keyframe animation (<c>.btimeline</c>) made for one prefab: values of the prefab's parts that
/// change over time. <see cref="LocalTime"/> maps a clock to a moment of the timeline,
/// <see cref="SampleAt"/> gives every track's value at that moment, and <see cref="Apply"/> writes those
/// values to the objects a <see cref="Binding"/> names.
/// </summary>
public sealed class Timeline
{
    public const string FormatName = "bayleaf.timeline";
    public const uint CurrentVersion = 1;
    public const string Extension = "btimeline";

    /// <summary>Longest timeline, in seconds.</summary>
    public const float MaxLength = 3600f;

    /// <summary>Most parts one timeline may animate.</summary>
    public const int MaxParts = 1024;

    /// <summary>Most tracks one part may have.</summary>
    public const int MaxTracks = 64;

    /// <summary>Most keys one track may have.</summary>
    public const int MaxKeys = 10_000;

    /// <summary>Most numbers one key's value may hold (a colour has four).</summary>
    public const int MaxNumbers = 16;

    /// <summary>Longest text value, in bytes.</summary>
    public const int MaxText = 1024;

    /// <summary>Keys closer in time than this are at the same moment.</summary>
    public const float SameTime = 0.0005f;

    public Timeline(string prefab, float length)
    {
        Prefab = prefab;
        Length = length;
    }

    public string Format { get; set; } = FormatName;

    public uint Version { get; set; } = CurrentVersion;

    /// <summary>The prefab it animates, as prefab links name it.</summary>
    public string Prefab { get; set; }

    /// <summary>Seconds, more than 0.</summary>
    public float Length { get; set; }

    /// <summary>Starts over at its end; otherwise it stays at its last moment.</summary>
    public bool Looping { get; set; }

    public List<Part> Parts { get; } = [];

    /// <summary>Reads a timeline file's text.</summary>
    public static Result<Timeline, string> FromJson(string text)
    {
        var parsed = JsonReader.Parse(text);
        var read = parsed.IsOk ? JsonRead.Try(() => Read(parsed.Value)) : Result.Fail(parsed.Error);
        if (!read.TryGet(out var timeline, out var error))
        {
            return Result.Fail($"not a timeline: {error}");
        }
        if (timeline.Format != FormatName)
        {
            return Result.Fail($"not a timeline (format \"{timeline.Format}\")");
        }
        if (timeline.Version is < 1 or > CurrentVersion)
        {
            return Result.Fail($"unsupported timeline version {timeline.Version}");
        }
        var valid = timeline.Validate();
        return valid.IsOk ? timeline : Result.Fail(valid.Error);
    }

    private static Timeline Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Timeline", true, "format", "version", "prefab", "length", "looping", "parts");
        var timeline = new Timeline(f.Required("prefab", JsonRead.Str), f.Required("length", JsonRead.F32))
        {
            Format = f.Required("format", JsonRead.Str),
            Version = f.Required("version", JsonRead.U32),
            Looping = f.Optional("looping", JsonRead.Bool, false),
        };
        timeline.Parts.AddRange(f.Optional("parts", v => JsonRead.List(v, Part.Read), []));
        return timeline;
    }

    /// <summary>The file's text.</summary>
    public string ToJson() => new JsonObject
    {
        { "format", J.S(Format) },
        { "version", J.U(Version) },
        { "prefab", J.S(Prefab) },
        { "length", J.F(Length) },
        { "looping", J.B(Looping) },
        { "parts", new JsonArray(Parts.Select(p => p.Write())) },
    }.ToPretty();

    /// <summary>Checks limits, finite times inside the length, keys in time order and values of a kind a track can hold.</summary>
    public Result<Unit, string> Validate()
    {
        var length = CheckLength(Length);
        if (length.IsError)
        {
            return length;
        }
        if (Prefab.Length == 0 || Text.Bytes(Prefab) > 1024 || Text.HasNul(Prefab))
        {
            return Checks.Fail("a timeline names its prefab, in 1 to 1024 bytes");
        }
        if (Parts.Count > MaxParts)
        {
            return Checks.Fail($"a timeline animates at most {MaxParts} parts");
        }
        for (var i = 0; i < Parts.Count; i++)
        {
            var part = Parts[i];
            if (Parts.Take(i).Any(p => p.Id == part.Id))
            {
                return Checks.Fail($"part {part.Id} appears twice");
            }
            if (part.Tracks.Count > MaxTracks)
            {
                return Checks.Fail($"a part has at most {MaxTracks} tracks");
            }
            for (var j = 0; j < part.Tracks.Count; j++)
            {
                var track = part.Tracks[j];
                var target = track.Target.Check();
                if (target.IsError)
                {
                    return target;
                }
                if (part.Tracks.Take(j).Any(t => t.Target.Equals(track.Target)))
                {
                    return Checks.Fail($"part {part.Id} has the track {track.Target.Describe()} twice");
                }
                if (track.Keys.Count > MaxKeys)
                {
                    return Checks.Fail($"a track has at most {MaxKeys} keys");
                }
                for (var k = 0; k < track.Keys.Count; k++)
                {
                    var key = track.Keys[k];
                    var checks = Checks.All(() => CheckTime(key.Time, Length), () => CheckValue(key.Value));
                    if (checks.IsError)
                    {
                        return checks;
                    }
                    if (k > 0 && key.Time - track.Keys[k - 1].Time <= SameTime)
                    {
                        return Checks.Fail($"the keys of {track.Target.Describe()} are not in time order, or two share a moment");
                    }
                }
            }
        }
        return Checks.Ok;
    }

    public Part? FindPart(StableId id) => Parts.Find(p => p.Id == id);

    /// <summary>The moment of the timeline a clock <paramref name="seconds"/> after it started shows: it wraps when the timeline loops and stops at the end when it plays once.</summary>
    public float LocalTime(double seconds)
    {
        var length = (double)Math.Max(Length, SameTime);
        var t = Looping ? ((seconds % length) + length) % length : Math.Clamp(seconds, 0, length);
        return (float)t;
    }

    /// <summary>Whether a timeline that plays once is over <paramref name="seconds"/> after it started.</summary>
    public bool Finished(double seconds) => !Looping && seconds >= Length;

    /// <summary>Every track's value at <paramref name="time"/> (seconds of the timeline), tracks without keys left out.</summary>
    public IReadOnlyList<Sample> SampleAt(float time) =>
        Parts.SelectMany(part => part.Tracks.Select(track => (track, value: track.ValueAt(time))).Where(t => t.value is not null).Select(t => new Sample(part.Id, t.track.Target, t.value!))).ToList();

    /// <summary>Checks a timeline length.</summary>
    public static Result<Unit, string> CheckLength(float length) =>
        float.IsFinite(length) && length > SameTime && length <= MaxLength ? Checks.Ok : Checks.Fail($"a timeline's length is more than 0 and at most {Text.Number(MaxLength)} seconds");

    /// <summary>Checks a key's time against the timeline's length.</summary>
    public static Result<Unit, string> CheckTime(float time, float length) =>
        float.IsFinite(time) && J.Within(time, 0f, length + SameTime) ? Checks.Ok : Checks.Fail($"a key's time is between 0 and the timeline's length ({Text.Number(length)} seconds)");

    /// <summary>Checks that a key's value is one a track can hold: a finite number, up to <see cref="MaxNumbers"/> finite numbers, true/false or text.</summary>
    public static Result<Unit, string> CheckValue(JsonValue value)
    {
        var ok = value switch
        {
            JsonNumber n => double.IsFinite(n.ToDouble()),
            JsonBool => true,
            JsonString s => Text.Bytes(s.Value) <= MaxText && !Text.HasNul(s.Value),
            JsonArray a => a.Count > 0 && a.Count <= MaxNumbers && a.All(v => v is JsonNumber n && double.IsFinite(n.ToDouble())),
            _ => false,
        };
        return ok ? Checks.Ok : Checks.Fail($"a key's value is a finite number, 1 to {MaxNumbers} finite numbers, true/false or text");
    }

    private static JsonValue Number(double v) => double.IsFinite(v) ? JsonNumber.Of(v) : JsonValue.Null;

    /// <summary>The value <paramref name="amount"/> (0 to 1) of the way from <paramref name="a"/> to <paramref name="b"/>. Numbers and lists of numbers of the same length blend; anything else holds <paramref name="a"/> until <paramref name="b"/>'s moment.</summary>
    public static JsonValue Blend(JsonValue a, JsonValue b, float amount)
    {
        var mixAmount = (double)amount;
        double Mix(double x, double y) => x + (y - x) * mixAmount;
        switch (a, b)
        {
            case (JsonNumber x, JsonNumber y):
                return Number(Mix(x.ToDouble(), y.ToDouble()));
            case (JsonArray xs, JsonArray ys) when xs.Count == ys.Count:
                if (xs.Zip(ys).All(p => p.First is JsonNumber && p.Second is JsonNumber))
                {
                    return new JsonArray(xs.Zip(ys).Select(p => Number(Mix(((JsonNumber)p.First).ToDouble(), ((JsonNumber)p.Second).ToDouble()))));
                }
                return a;
        }
        return mixAmount >= 1.0 ? b : a;
    }

    /// <summary>The current value of <paramref name="target"/> on <paramref name="obj"/>, as a track holds it.</summary>
    public static JsonValue? ReadValue(Scene scene, StableId obj, Target target)
    {
        switch (target)
        {
            case Target.Field f:
                var component = scene.Component(obj, f.Component);
                return component.IsOk ? FieldOf(component.Value, f.Name) : null;
            case Target.ScriptProperty p:
                if (!scene.TryGet<Scripts>(obj, out var scripts))
                {
                    return null;
                }
                var entry = scripts.Items.FirstOrDefault(s => s.Module == p.Script);
                return entry is not null && entry.Properties.TryGetValue(p.Name, out var value) ? value.Write().Canonical() : null;
            default:
                return null;
        }
    }

    /// <summary>The field at a dotted path inside a component's value.</summary>
    public static JsonValue? FieldOf(JsonValue value, string path)
    {
        JsonValue? current = value;
        foreach (var name in path.Split('.'))
        {
            current = current?.Get(name);
        }
        return current;
    }

    /// <summary>Replaces the field at a dotted path; false when a step is missing.</summary>
    private static bool SetField(JsonValue value, string path, Func<JsonValue, JsonValue> change)
    {
        var names = path.Split('.');
        var current = value;
        for (var i = 0; i < names.Length - 1; i++)
        {
            if (current.Get(names[i]) is not { } next)
            {
                return false;
            }
            current = next;
        }
        if (current is not JsonObject o || o.Find(names[^1]) is not { } slot)
        {
            return false;
        }
        o.Set(names[^1], change(slot));
        return true;
    }

    /// <summary>
    /// Writes <paramref name="samples"/> to the objects <paramref name="binding"/> names, each component
    /// once. Parts with no object, fields the object lacks and values its component refuses are skipped;
    /// what went wrong is returned, once per track.
    /// </summary>
    public static IReadOnlyList<string> Apply(Scene scene, Binding binding, IReadOnlyList<Sample> samples)
    {
        var problems = new List<string>();
        var changed = new SortedDictionary<(StableId Object, string Component), JsonValue>(Comparer<(StableId, string)>.Create((x, y) =>
        {
            var order = x.Item1.CompareTo(y.Item1);
            return order != 0 ? order : Utf8Order.Instance.Compare(x.Item2, y.Item2);
        }));
        var scripts = new SortedDictionary<StableId, Scripts>();
        string NameOf(StableId obj) => scene.NameOf(obj) ?? "Object";
        foreach (var sample in samples)
        {
            if (binding.ObjectOf(sample.Part) is not { } obj)
            {
                continue;
            }
            switch (sample.Target)
            {
                case Target.Field f:
                    var key = (obj, f.Component);
                    if (!changed.TryGetValue(key, out var componentValue))
                    {
                        var current = scene.Component(obj, f.Component);
                        if (current.IsError)
                        {
                            problems.Add($"{NameOf(obj)} has no {f.Component} component");
                            continue;
                        }
                        componentValue = current.Value;
                        changed[key] = componentValue;
                    }
                    if (!SetField(componentValue, f.Name, slot => Fit(slot, sample.Value)))
                    {
                        problems.Add($"the {f.Component} component has no field {f.Name}");
                    }
                    break;
                case Target.ScriptProperty p:
                    if (!scripts.TryGetValue(obj, out var objectScripts))
                    {
                        if (!scene.TryGet<Scripts>(obj, out var current))
                        {
                            problems.Add($"{NameOf(obj)} has no scripts");
                            continue;
                        }
                        objectScripts = new Scripts(current.Items.Select(s => new ScriptRef(s.Module, new SortedDictionary<string, PropertyValue>(s.Properties, Utf8Order.Instance))).ToList());
                        scripts[obj] = objectScripts;
                    }
                    var entry = objectScripts.Items.FirstOrDefault(s => s.Module == p.Script);
                    if (entry is null)
                    {
                        problems.Add($"{NameOf(obj)} has no script {p.Script}");
                        continue;
                    }
                    var read = JsonRead.Try(() => PropertyValue.Read(sample.Value));
                    if (read.IsOk)
                    {
                        entry.Properties[p.Name] = read.Value;
                    }
                    else
                    {
                        problems.Add($"{p.Name} of {p.Script} cannot hold {sample.Value}");
                    }
                    break;
            }
        }
        foreach (var ((obj, component), value) in changed)
        {
            var set = scene.SetComponent(obj, component, value);
            if (set.IsError)
            {
                problems.Add($"{component} of {NameOf(obj)}: {set.Error}");
            }
        }
        foreach (var (obj, value) in scripts)
        {
            var set = scene.Set(obj, value);
            if (set.IsError)
            {
                problems.Add($"scripts of {NameOf(obj)}: {set.Error}");
            }
        }
        return problems;
    }

    /// <summary><paramref name="value"/> in the shape of the field it goes into: a colour key of three numbers keeps the field's alpha, a whole number stays whole.</summary>
    private static JsonValue Fit(JsonValue current, JsonValue value)
    {
        switch (current, value)
        {
            case (JsonArray now, JsonArray next) when next.Count < now.Count:
                return new JsonArray(next.Concat(now.Skip(next.Count)));
            case (JsonNumber now, JsonNumber next) when now.IsInteger:
                var rounded = Math.Round(next.ToDouble(), MidpointRounding.AwayFromZero);
                if (double.IsNaN(rounded))
                {
                    return value;
                }
                return rounded >= 0 && now.NumberKind == NumberKind.Natural ? JsonNumber.Of((ulong)rounded) : JsonNumber.Of((long)rounded);
            default:
                return value;
        }
    }
}

/// <summary>Which object of a scene stands for each part of a timeline.</summary>
public sealed class Binding
{
    public SortedDictionary<StableId, StableId> Objects { get; } = [];

    /// <summary>Parts whose ids are the scene's own (the prefab open on its own).</summary>
    public static Binding SameIds(Scene scene, Timeline timeline)
    {
        var binding = new Binding();
        foreach (var part in timeline.Parts.Where(p => scene.Contains(p.Id)))
        {
            binding.Objects[part.Id] = part.Id;
        }
        return binding;
    }

    /// <summary>Parts of a placed copy whose root is <paramref name="root"/>: the prefab file lists its objects in hierarchy order, and the copy holds them in the same order.</summary>
    public static Binding ByOrder(Scene scene, StableId root, IReadOnlyList<StableId> fileIds, Timeline timeline)
    {
        var exported = scene.Export([root]);
        var copy = exported.IsOk ? exported.Value.Select(o => o.Id).ToList() : [];
        var binding = new Binding();
        foreach (var part in timeline.Parts)
        {
            var at = fileIds.ToList().IndexOf(part.Id);
            if (at >= 0 && at < copy.Count)
            {
                binding.Objects[part.Id] = copy[at];
            }
        }
        return binding;
    }

    public StableId? ObjectOf(StableId part) => Objects.TryGetValue(part, out var obj) ? obj : null;

    /// <summary>The part <paramref name="obj"/> stands for.</summary>
    public StableId? PartOf(StableId obj) => Objects.Where(p => p.Value == obj).Select(p => (StableId?)p.Key).FirstOrDefault();
}
