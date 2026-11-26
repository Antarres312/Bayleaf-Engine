using System.Numerics;

using Bayleaf.Assets;
using Bayleaf.Core;

namespace Bayleaf.Animation;

/// <summary>A motion playing, and how far it has faded in.</summary>
internal sealed class Entry(Motion motion, float time, float fade)
{
    public Motion Motion { get; } = motion;

    /// <summary>Seconds since it started.</summary>
    public float Time { get; set; } = time;

    /// <summary>Seconds it takes to fade in; 0 for at once.</summary>
    public float Fade { get; } = fade;

    public float Weight => Fade <= 0f ? 1f : Math.Clamp(Time / Fade, 0f, 1f);
}

/// <summary>The motions playing on the whole skeleton (the base layer) or on one joint and the nodes under it, over the base.</summary>
public sealed class Layer
{
    internal Layer(string joint)
    {
        Joint = joint;
    }

    /// <summary>The joint the layer plays on, with every node under it; empty for the base layer.</summary>
    public string Joint { get; }

    /// <summary>What it played and plays, oldest first: each fades in over the ones before.</summary>
    internal List<Entry> Entries { get; } = [];

    /// <summary>While a layer over the base stops: seconds left of its fade out, and in all.</summary>
    internal (float Left, float Total)? FadeOut { get; set; }

    /// <summary>What plays now (the newest motion), and seconds since it started.</summary>
    public (Motion Motion, float Time)? Current => Entries.Count > 0 ? (Entries[^1].Motion, Entries[^1].Time) : null;

    /// <summary>Whether a fade is under way: in from what played before, or out of the whole layer.</summary>
    public bool Fading => FadeOut is not null || Entries.Count > 1;

    /// <summary>How much of the layer shows (a layer over the base fades out as it stops).</summary>
    internal float Strength => FadeOut is { } f ? (f.Total > 0f ? Math.Clamp(f.Left / f.Total, 0f, 1f) : 0f) : 1f;

    /// <summary>Each entry's share of the layer's pose: its weight times what the later ones leave.</summary>
    internal float[] Shares()
    {
        var shares = new float[Entries.Count];
        var left = 1f;
        for (var i = Entries.Count - 1; i >= 0; i--)
        {
            var w = i == 0 ? 1f : Entries[i].Weight;
            shares[i] = left * w;
            left *= 1f - w;
        }
        return shares;
    }

    internal Pose? PoseOf(Rig rig)
    {
        Pose? pose = null;
        foreach (var e in Entries)
        {
            var own = e.Motion.PoseAt(rig, e.Time);
            pose = pose is null ? own : pose.Mix(own, e.Weight);
        }
        return pose;
    }
}

/// <summary>What one step of a player gave.</summary>
public sealed class PlayerStep
{
    /// <summary>The clip events it passed, in order.</summary>
    public List<string> Events { get; } = [];

    /// <summary>
    /// How far the root joint moved along the ground, in the model's space (with its import scale
    /// and axes), when the player follows root motion.
    /// </summary>
    public Vector3 Root { get; set; }
}

/// <summary>
/// The motions playing on one object, by layer (the base first), and the parameters they read. It
/// fades one motion into the next, keeps layers, and runs the object's animation states (a
/// <see cref="Machine"/>, compiled from a <c>.banim</c> graph). Each step gives the clip events it
/// passed and the root joint's movement.
/// </summary>
public sealed class Player
{
    /// <summary>Longest fade, in seconds.</summary>
    public const float MaxFade = 60f;

    /// <summary>Most layers one object plays at once.</summary>
    public const int MaxLayers = 16;

    /// <summary>Most parameters one object holds.</summary>
    public const int MaxParameters = 256;

    private readonly List<Layer> layers = [new("")];

    /// <summary>The animation states it runs on the base layer, and the state it is in.</summary>
    private (Machine Machine, int State)? states;

    /// <summary>The parameters mixes, layers and animation states read, by name.</summary>
    public SortedDictionary<string, Parameter> Parameters { get; } = new(Utf8Order.Instance);

    /// <summary>
    /// The joint whose movement along the ground moves the object, drawn in place: root motion.
    /// Null plays every motion where it was made.
    /// </summary>
    public int? Root { get; set; }

    /// <summary>The base layer, then the layers over it.</summary>
    public IReadOnlyList<Layer> Layers => layers;

    /// <summary>The layer on <paramref name="joint"/> (empty for the base).</summary>
    public Layer? LayerOn(string joint) => layers.Find(l => l.Joint == joint);

    private static float FadeOf(float fade) => float.IsFinite(fade) ? Math.Clamp(fade, 0f, MaxFade) : 0f;

    /// <summary>
    /// Plays <paramref name="motion"/> on the layer of <paramref name="joint"/> (empty: the whole
    /// skeleton), fading it in over <paramref name="fade"/> seconds from what played there; a new
    /// layer starts at full strength at once.
    /// </summary>
    public Result<Unit, string> Play(string joint, Motion motion, float fade)
    {
        fade = FadeOf(fade);
        var layer = LayerOn(joint);
        if (layer is null)
        {
            if (layers.Count >= MaxLayers)
            {
                return Result.Fail($"an object plays at most {MaxLayers} layers");
            }
            layer = new Layer(joint);
            layers.Add(layer);
        }
        // A layer that was stopping comes back at once.
        layer.FadeOut = null;
        layer.Entries.Add(new Entry(motion, 0f, layer.Entries.Count == 0 ? 0f : fade));
        // A fade over many quick changes keeps the last few.
        if (layer.Entries.Count > 8)
        {
            layer.Entries.RemoveAt(0);
        }
        return Result.Ok<string>();
    }

    /// <summary>Stops the layer of <paramref name="joint"/> over <paramref name="fade"/> seconds: the base layer fades to the rest pose, a layer over it fades out and goes.</summary>
    public void Stop(string joint, float fade)
    {
        fade = FadeOf(fade);
        if (joint.Length == 0)
        {
            var b = layers[0];
            if (b.Entries.Count > 0)
            {
                b.Entries.Add(new Entry(Motion.AtRest, 0f, fade));
            }
            return;
        }
        if (fade <= 0f)
        {
            layers.RemoveAll(l => l.Joint == joint);
        }
        else if (LayerOn(joint) is { } layer)
        {
            layer.FadeOut = (fade, fade);
        }
    }

    /// <summary>Puts what plays on the layer of <paramref name="joint"/> at <paramref name="time"/> seconds since it started, ending any fade into it (scrubbing a preview).</summary>
    public void Seek(string joint, float time)
    {
        time = float.IsFinite(time) ? Math.Max(time, 0f) : 0f;
        if (LayerOn(joint) is not { } layer || layer.Entries.Count == 0)
        {
            return;
        }
        var last = layer.Entries[^1];
        layer.Entries.Clear();
        layer.Entries.Add(new Entry(last.Motion, time, 0f));
    }

    /// <summary>Sets a parameter (a number, a true/false or a trigger).</summary>
    public Result<Unit, string> SetParameter(string name, Parameter value)
    {
        if (!Parameters.ContainsKey(name) && Parameters.Count >= MaxParameters)
        {
            return Result.Fail($"an object holds at most {MaxParameters} animation parameters");
        }
        Parameters[name] = value;
        return Result.Ok<string>();
    }

    /// <summary>Runs <paramref name="machine"/> on the base layer from its start state (in place of what played there).</summary>
    public void RunStates(Machine machine)
    {
        var start = machine.Start;
        if (start >= 0 && start < machine.States.Count)
        {
            // The base layer is always there, so playing on it cannot fail.
            _ = Play("", machine.States[start].Motion, 0f);
        }
        states = (machine, start);
    }

    /// <summary>The animation states it runs, if any.</summary>
    public Machine? Machine => states?.Machine;

    /// <summary>The name of the state it is in, when it runs animation states.</summary>
    public string? State => states is { } s && s.State >= 0 && s.State < s.Machine.States.Count ? s.Machine.States[s.State].Name : null;

    private static Motion MotionOf(Machine machine, int state) => state >= 0 && state < machine.States.Count ? machine.States[state].Motion : Motion.AtRest;

    /// <summary>Goes to the state called <paramref name="name"/> now, fading over <paramref name="fade"/> seconds.</summary>
    public Result<Unit, string> GoTo(string name, float fade)
    {
        if (states is not { } s)
        {
            return Result.Fail("the object runs no animation states");
        }
        if (s.Machine.State(name) is not { } index)
        {
            return Result.Fail($"there is no state named \"{name}\"; there are {string.Join(", ", s.Machine.States.Select(st => st.Name))}");
        }
        var played = Play("", MotionOf(s.Machine, index), fade);
        if (played.IsError)
        {
            return played;
        }
        states = (s.Machine, index);
        return played;
    }

    /// <summary>Takes the first transition out of the current state whose condition holds, using up the triggers it read.</summary>
    private void Transition(Skeleton skeleton, Clips clips)
    {
        if (states is not { } s)
        {
            return;
        }
        var rig = new Rig(skeleton, clips, Parameters);
        var inState = default(InState);
        if (layers[0].Current is { } current)
        {
            var duration = current.Motion.Duration(rig);
            inState = new InState(current.Time, duration > 0f ? current.Time / duration : 0f);
        }
        if (s.Machine.Next(s.State, Parameters, inState) is not { } transition)
        {
            return;
        }
        var used = new List<string>();
        transition.When.Triggers(used);
        foreach (var name in used)
        {
            Parameters[name] = Parameter.Trigger(false);
        }
        // The base layer is always there, so playing on it cannot fail.
        _ = Play("", MotionOf(s.Machine, transition.To), transition.Fade);
        states = (s.Machine, transition.To);
    }

    /// <summary>
    /// Moves every motion on by <paramref name="dt"/> seconds: gives the clip events passed (of the
    /// base layer's main motion, and of layers over it) and, with root motion, how far the root joint moved.
    /// </summary>
    public PlayerStep Advance(Skeleton skeleton, Clips clips, float dt)
    {
        dt = float.IsFinite(dt) ? Math.Max(dt, 0f) : 0f;
        Transition(skeleton, clips);
        var step = new PlayerStep();
        var rig = new Rig(skeleton, clips, Parameters);
        for (var index = 0; index < layers.Count; index++)
        {
            var layer = layers[index];
            var shares = layer.Shares();
            var main = Numbers.IndexOfMax(shares);
            for (var i = 0; i < layer.Entries.Count; i++)
            {
                var e = layer.Entries[i];
                if (i == main && layer.FadeOut is null)
                {
                    e.Motion.Events(rig, e.Time, e.Time + dt, step.Events);
                }
                if (index == 0 && Root is { } root)
                {
                    step.Root += e.Motion.RootMotion(rig, root, e.Time, e.Time + dt) * (i < shares.Length ? shares[i] : 0f);
                }
            }
            foreach (var e in layer.Entries)
            {
                e.Time += dt;
            }
            // What a finished fade covers no longer shows.
            var lastFull = layer.Entries.FindLastIndex(e => e.Weight >= 1f);
            if (lastFull > 0)
            {
                layer.Entries.RemoveRange(0, lastFull);
            }
            if (layer.FadeOut is { } f)
            {
                layer.FadeOut = (f.Left - dt, f.Total);
            }
        }
        layers.RemoveAll(l => l.Joint.Length > 0 && l.FadeOut is { } f && !(f.Left > 0f));
        return step;
    }

    /// <summary>
    /// The pose the player gives now: the base layer's, with each layer over it on its joint and the
    /// nodes under it; with root motion, the root joint kept in place along the ground.
    /// </summary>
    public Pose Pose(Skeleton skeleton, Clips clips)
    {
        var rig = new Rig(skeleton, clips, Parameters);
        var pose = layers[0].PoseOf(rig) ?? skeleton.RestPose();
        if (Root is { } root)
        {
            Motion.PinRoot(skeleton, pose, root);
        }
        foreach (var layer in layers.Skip(1))
        {
            if (layer.PoseOf(rig) is not { } over || skeleton.Mask(layer.Joint) is not { } mask)
            {
                continue;
            }
            pose = pose.MixMasked(over, layer.Strength, mask);
        }
        return pose;
    }

    /// <summary>Whether anything plays (the base layer has a motion other than rest, or a layer is over it).</summary>
    public bool IsPlaying => layers.Count > 1 || layers[0].Current is { Motion: not Motion.Rest };
}
