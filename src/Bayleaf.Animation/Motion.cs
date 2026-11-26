using System.Numerics;

using Bayleaf.Assets;

namespace Bayleaf.Animation;

/// <summary>What kind of value a parameter holds.</summary>
public enum ParameterKind
{
    Number,
    Flag,

    /// <summary>True from when it is set until a transition it lets through uses it up.</summary>
    Trigger,
}

public static class ParameterKinds
{
    /// <summary>The kind's name in messages.</summary>
    public static string Label(this ParameterKind kind) => kind switch
    {
        ParameterKind.Number => "number",
        ParameterKind.Flag => "true/false",
        _ => "trigger",
    };
}

/// <summary>A value scripts set on an object's animation: what mixes, layers and animation states read.</summary>
public readonly record struct Parameter
{
    private Parameter(ParameterKind kind, float number, bool set)
    {
        Kind = kind;
        Value = number;
        IsSet = set;
    }

    public ParameterKind Kind { get; }

    /// <summary>A number parameter's value.</summary>
    private float Value { get; }

    /// <summary>A true/false or trigger parameter's value.</summary>
    private bool IsSet { get; }

    public static Parameter Number(float value) => new(ParameterKind.Number, value, false);

    public static Parameter Flag(bool value) => new(ParameterKind.Flag, 0f, value);

    public static Parameter Trigger(bool value) => new(ParameterKind.Trigger, 0f, value);

    /// <summary>As a number: a true/false or a trigger reads 1 when set, else 0.</summary>
    public float AsNumber() => Kind == ParameterKind.Number ? Value : IsSet ? 1f : 0f;

    /// <summary>As true/false: a number is true when it is not 0.</summary>
    public bool AsFlag() => Kind == ParameterKind.Number ? Value != 0f : IsSet;

    public override string ToString() => Kind == ParameterKind.Number ? $"Number({Value})" : $"{Kind}({IsSet})";
}

/// <summary>What a number in a motion reads: a value, or a parameter by name (0 while none of that name is set).</summary>
public readonly record struct Number
{
    private Number(float value, string? parameter)
    {
        Value = value;
        Parameter = parameter;
    }

    public float Value { get; }

    /// <summary>The parameter it reads; null for a value typed in.</summary>
    public string? Parameter { get; }

    public static Number Of(float value) => new(value, null);

    public static Number Named(string parameter) => new(0f, parameter);

    public float Read(IReadOnlyDictionary<string, Parameter> parameters)
    {
        var n = Parameter is { } name ? (parameters.TryGetValue(name, out var p) ? p.AsNumber() : 0f) : Value;
        return float.IsFinite(n) ? n : 0f;
    }
}

/// <summary>One motion of a mix, and its point.</summary>
public readonly record struct MixItem(Motion Motion, float At);

/// <summary>What a motion plays on: the skeleton, the clips it names, and the parameters its numbers read.</summary>
public readonly record struct Rig(Skeleton Skeleton, Clips Clips, IReadOnlyDictionary<string, Parameter> Parameters);

/// <summary>What plays on a skeleton.</summary>
public abstract record Motion
{
    /// <summary>Most loops of a clip one step looks through for events and root motion.</summary>
    private const long MaxLoops = 64;

    private Motion()
    {
    }

    /// <summary>Every node at rest.</summary>
    public static readonly Motion AtRest = new Rest();

    /// <summary>Every node at rest.</summary>
    public sealed record Rest : Motion;

    /// <summary>
    /// A clip by name, <paramref name="Speed"/> times as fast as made, from its end round to its
    /// start when <paramref name="Looping"/>, else holding its last moment.
    /// </summary>
    public sealed record Clip(string Name, float Speed, bool Looping) : Motion;

    /// <summary>
    /// Motions by where the number <paramref name="By"/> lies among their points: between two
    /// points, the two round it, each by how near it is; before the first or past the last, that one
    /// alone (walk at 1 and run at 3, by speed). They play in step: each reaches its end when the mix does.
    /// </summary>
    public sealed record Mix(IReadOnlyList<MixItem> Items, Number By) : Motion;

    /// <summary>
    /// <paramref name="Top"/> over <paramref name="Base"/> on the joint named <paramref name="Joint"/>
    /// and every node under it (the whole skeleton when it is empty), by <paramref name="Weight"/> from 0 to 1.
    /// </summary>
    public sealed record Layer(Motion Base, Motion Top, string Joint, Number Weight) : Motion;

    /// <summary>A clip at its own speed, looping.</summary>
    public static Motion OfClip(string name) => new Clip(name, 1f, true);

    /// <summary>The weight of each point in <paramref name="points"/> for <paramref name="value"/>: see <see cref="Mix"/>.</summary>
    internal static float[] MixWeights(IReadOnlyList<float> points, float value)
    {
        var weights = new float[points.Count];
        int? below = null, above = null;
        for (var i = 0; i < points.Count; i++)
        {
            var p = points[i];
            // The largest point at or below (the last of equal ones), the smallest at or above (the first).
            if (p <= value && (below is not { } b || Numbers.TotalCmp(p, points[b]) >= 0))
            {
                below = i;
            }
            if (p >= value && (above is not { } a || Numbers.TotalCmp(p, points[a]) < 0))
            {
                above = i;
            }
        }
        if (below is { } lo && above is { } hi && hi != lo && points[hi] > points[lo])
        {
            var t = (value - points[lo]) / (points[hi] - points[lo]);
            weights[lo] = 1f - t;
            weights[hi] = t;
        }
        else if ((below ?? above) is { } only)
        {
            weights[only] = 1f;
        }
        return weights;
    }

    /// <summary>Where a clip of <paramref name="duration"/> seconds is <paramref name="time"/> seconds after it started at <paramref name="speed"/>.</summary>
    internal static float ClipTime(float duration, float speed, bool looping, float time)
    {
        var t = time * speed;
        if (duration <= 0f || !float.IsFinite(t))
        {
            return 0f;
        }
        return looping ? Numbers.RemEuclid(t, duration) : Math.Clamp(t, 0f, duration);
    }

    private static Matrix4x4 Local(Pose pose, int node) => node >= 0 && node < pose.Locals.Length ? pose.Locals[node].Matrix() : Matrix4x4.Identity;

    private static int? ParentOf(Skeleton skeleton, int node) => node >= 0 && node < skeleton.Nodes.Count ? skeleton.Nodes[node].Parent : null;

    /// <summary>The matrix of <paramref name="node"/> into the model (before the import scale and axes) in <paramref name="pose"/>, by its chain of parents only.</summary>
    internal static Matrix4x4 NodeMatrix(Skeleton skeleton, Pose pose, int node)
    {
        var m = Local(pose, node);
        var parent = ParentOf(skeleton, node);
        var steps = 0;
        while (parent is { } p)
        {
            m *= Local(pose, p);
            parent = ParentOf(skeleton, p);
            steps++;
            if (steps > skeleton.Nodes.Count)
            {
                break;
            }
        }
        return m;
    }

    /// <summary>Where <paramref name="node"/> is in the model, with the import scale and axes (so Y is up), in <paramref name="pose"/>.</summary>
    internal static Vector3 ModelPosition(Skeleton skeleton, Pose pose, int node) => (NodeMatrix(skeleton, pose, node) * skeleton.Root).Translation;

    /// <summary>The joint root motion follows when none is named: the first node, parents first, that one of the model's animations moves.</summary>
    public static int? DefaultRoot(Skeleton skeleton)
    {
        bool Moved(int node) => skeleton.Animations.Any(a => a.Channels.Any(c => c.Node == node && c.Property == ChannelProperty.Translation));
        foreach (var node in skeleton.Order)
        {
            if (Moved(node))
            {
                return node;
            }
        }
        return null;
    }

    /// <summary>
    /// Keeps joint <paramref name="root"/> in place along the ground in <paramref name="pose"/>: moves
    /// it back by how far it is, along X and Z of the model, from where it stands at rest. Its height stays.
    /// </summary>
    public static void PinRoot(Skeleton skeleton, Pose pose, int root)
    {
        var rest = skeleton.RestPose();
        var offset = ModelPosition(skeleton, pose, root) - ModelPosition(skeleton, rest, root);
        offset.Y = 0f;
        var parent = ParentOf(skeleton, root) is { } p ? NodeMatrix(skeleton, pose, p) : Matrix4x4.Identity;
        if (!Matrix4x4.Invert(parent * skeleton.Root, out var intoParent) || !Numbers.IsFinite(intoParent))
        {
            return;
        }
        if (root >= 0 && root < pose.Locals.Length)
        {
            ref var local = ref pose.Locals[root];
            local = local with { Translation = local.Translation - Vector3.TransformNormal(offset, intoParent) };
        }
    }

    private static float[] Points(IReadOnlyList<MixItem> items) => [.. items.Select(i => i.At)];

    /// <summary>Seconds from its start to its end as it plays now (a mix's by its weights).</summary>
    public float Duration(Rig rig)
    {
        switch (this)
        {
            case Clip c:
                var d = rig.Clips.Get(c.Name)?.Duration ?? 0f;
                return c.Speed > 1e-6f ? d / c.Speed : d;
            case Mix m:
                var weights = MixWeights(Points(m.Items), m.By.Read(rig.Parameters));
                var sum = 0f;
                for (var i = 0; i < m.Items.Count; i++)
                {
                    sum += weights[i] > 0f ? m.Items[i].Motion.Duration(rig) * weights[i] : 0f;
                }
                return sum;
            case Layer l:
                return l.Base.Duration(rig);
            default:
                return 0f;
        }
    }

    /// <summary>Whether it starts over at its end (a mix or layer when any of its clips does).</summary>
    public bool IsLooping => this switch
    {
        Clip c => c.Looping,
        Mix m => m.Items.Any(i => i.Motion.IsLooping),
        Layer l => l.Base.IsLooping || l.Top.IsLooping,
        _ => false,
    };

    /// <summary>For each part of a mix, its weight and the time it is at when the mix is at <paramref name="time"/>.</summary>
    private (float Weight, float At)[] MixParts(Mix mix, Rig rig, float time)
    {
        var weights = MixWeights(Points(mix.Items), mix.By.Read(rig.Parameters));
        var whole = Duration(rig);
        var parts = new (float, float)[mix.Items.Count];
        for (var i = 0; i < parts.Length; i++)
        {
            var own = mix.Items[i].Motion.Duration(rig);
            parts[i] = (weights[i], whole > 0f ? time * own / whole : time);
        }
        return parts;
    }

    private static void CopyInto(Pose from, Pose to) => Array.Copy(from.Locals, to.Locals, Math.Min(from.Locals.Length, to.Locals.Length));

    /// <summary>Writes how it poses the skeleton <paramref name="time"/> seconds after it started over <paramref name="pose"/>: start from the rest pose.</summary>
    public void Sample(Rig rig, float time, Pose pose)
    {
        switch (this)
        {
            case Clip c:
                if (rig.Clips.Get(c.Name) is { } source)
                {
                    source.Sample(rig.Skeleton, ClipTime(source.Duration, c.Speed, c.Looping, time), pose);
                }
                break;
            case Mix m:
                var parts = MixParts(m, rig, time);
                Pose? mixed = null;
                var total = 0f;
                for (var i = 0; i < parts.Length; i++)
                {
                    var (weight, at) = parts[i];
                    if (weight <= 0f)
                    {
                        continue;
                    }
                    var own = rig.Skeleton.RestPose();
                    m.Items[i].Motion.Sample(rig, at, own);
                    // Each further pose by its share of the weight so far.
                    mixed = mixed is null ? own : mixed.Mix(own, weight / (total + weight));
                    total += weight;
                }
                if (mixed is not null)
                {
                    CopyInto(mixed, pose);
                }
                break;
            case Layer l:
                l.Base.Sample(rig, time, pose);
                if (rig.Skeleton.Mask(l.Joint) is not { } mask)
                {
                    return;
                }
                var over = rig.Skeleton.RestPose();
                l.Top.Sample(rig, time, over);
                CopyInto(pose.MixMasked(over, l.Weight.Read(rig.Parameters), mask), pose);
                break;
            default:
                break;
        }
    }

    /// <summary>The pose <paramref name="time"/> seconds after it started.</summary>
    public Pose PoseAt(Rig rig, float time)
    {
        var pose = rig.Skeleton.RestPose();
        Sample(rig, time, pose);
        return pose;
    }

    /// <summary>
    /// The names of the clip events it passes from <paramref name="from"/> (included) to
    /// <paramref name="to"/> (not) seconds after it started, in order: of a mix, those of its
    /// heaviest part; of a layer, those of its base and, at a weight of a half or more, of its top.
    /// </summary>
    public void Events(Rig rig, float from, float to, List<string> output)
    {
        if (to <= from)
        {
            return;
        }
        switch (this)
        {
            case Clip c:
                if (rig.Clips.Get(c.Name) is not { } source)
                {
                    return;
                }
                var (d, events) = (source.Duration, source.Events);
                if (events.Count == 0 || c.Speed <= 0f)
                {
                    return;
                }
                var (a, b) = (from * c.Speed, to * c.Speed);
                if (!c.Looping || d <= 0f)
                {
                    output.AddRange(events.Where(e => e.Time >= a && e.Time < b).Select(e => e.Name));
                    return;
                }
                var (first, last) = ((long)MathF.Floor(a / d), (long)MathF.Floor(b / d));
                for (var round = first; round <= Math.Min(last, first + MaxLoops); round++)
                {
                    foreach (var e in events)
                    {
                        var at = round * d + e.Time;
                        if (at >= a && at < b)
                        {
                            output.Add(e.Name);
                        }
                    }
                }
                break;
            case Mix m:
                var (start, end) = (MixParts(m, rig, from), MixParts(m, rig, to));
                if (Numbers.IndexOfMax([.. start.Select(s => s.Weight)]) is { } heaviest && heaviest < m.Items.Count && heaviest < end.Length)
                {
                    m.Items[heaviest].Motion.Events(rig, start[heaviest].At, end[heaviest].At, output);
                }
                break;
            case Layer l:
                l.Base.Events(rig, from, to, output);
                if (l.Weight.Read(rig.Parameters) >= 0.5f)
                {
                    l.Top.Events(rig, from, to, output);
                }
                break;
            default:
                break;
        }
    }

    /// <summary>
    /// How far joint <paramref name="root"/> moves along the ground (X and Z of the model, with its
    /// import scale and axes) from <paramref name="from"/> to <paramref name="to"/> seconds after it
    /// started: across each start over of a looping clip, the moves of the clip's runs added up. A
    /// mix moves it by its parts' weights; a layer as its base does.
    /// </summary>
    public Vector3 RootMotion(Rig rig, int root, float from, float to)
    {
        if (to <= from)
        {
            return Vector3.Zero;
        }
        var delta = Vector3.Zero;
        switch (this)
        {
            case Clip c:
                if (rig.Clips.Get(c.Name) is not { } source)
                {
                    return Vector3.Zero;
                }
                var d = source.Duration;
                Vector3 At(float t)
                {
                    var pose = rig.Skeleton.RestPose();
                    source.Sample(rig.Skeleton, t, pose);
                    return ModelPosition(rig.Skeleton, pose, root);
                }
                var (a, b) = (from * c.Speed, to * c.Speed);
                if (d <= 0f)
                {
                    delta = Vector3.Zero;
                }
                else if (!c.Looping)
                {
                    delta = At(Math.Clamp(b, 0f, d)) - At(Math.Clamp(a, 0f, d));
                }
                else
                {
                    var (first, last) = ((long)MathF.Floor(a / d), (long)MathF.Floor(b / d));
                    if (first == last)
                    {
                        delta = At(b - last * d) - At(a - first * d);
                    }
                    else
                    {
                        var run = At(d) - At(0f);
                        float whole = Math.Clamp(last - first - 1, 0, MaxLoops);
                        delta = (At(d) - At(a - first * d)) + run * whole + (At(b - last * d) - At(0f));
                    }
                }
                break;
            case Mix m:
                var (start, end) = (MixParts(m, rig, from), MixParts(m, rig, to));
                for (var i = 0; i < m.Items.Count; i++)
                {
                    var (w, at) = start[i];
                    delta += w > 0f ? m.Items[i].Motion.RootMotion(rig, root, at, end[i].At) * w : Vector3.Zero;
                }
                break;
            case Layer l:
                delta = l.Base.RootMotion(rig, root, from, to);
                break;
            default:
                break;
        }
        delta.Y = 0f;
        return delta;
    }

    /// <summary>What it names that the skeleton and clips do not have: clips and joints, one line each.</summary>
    public IReadOnlyList<string> Problems(Skeleton skeleton, Clips clips)
    {
        var found = new List<string>();
        GatherProblems(skeleton, clips, found);
        // Repeats one after another go, as Rust's `dedup` drops them.
        return [.. found.Where((p, i) => i == 0 || p != found[i - 1])];
    }

    private void GatherProblems(Skeleton skeleton, Clips clips, List<string> output)
    {
        switch (this)
        {
            case Clip c:
                if (clips.Get(c.Name) is null)
                {
                    var names = clips.Names.ToList();
                    output.Add($"there is no clip named \"{c.Name}\"; the model has {(names.Count == 0 ? "none" : string.Join(", ", names))}");
                }
                break;
            case Mix m:
                foreach (var item in m.Items)
                {
                    item.Motion.GatherProblems(skeleton, clips, output);
                }
                break;
            case Layer l:
                if (skeleton.Mask(l.Joint) is null)
                {
                    output.Add($"there is no joint named \"{l.Joint}\"");
                }
                l.Base.GatherProblems(skeleton, clips, output);
                l.Top.GatherProblems(skeleton, clips, output);
                break;
            default:
                break;
        }
    }
}
