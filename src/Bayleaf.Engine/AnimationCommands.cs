using System.Numerics;
using System.Text;

using Bayleaf.Animation;
using Bayleaf.Assets;
using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Scene.Components;

using Mesh = Bayleaf.Scene.Components.Mesh;
using ModelAnimation = Bayleaf.Scene.Components.ModelAnimation;
using Motion = Bayleaf.Animation.Motion;
using Number = Bayleaf.Animation.Number;
using Skeleton = Bayleaf.Assets.Skeleton;
using Transform = Bayleaf.Scene.Components.Transform;

namespace Bayleaf.Engine;

/// <summary>What an object's preview shows (see <see cref="Session.AnimationPreview"/>).</summary>
/// <param name="Clip">The clip it shows.</param>
/// <param name="Time">Seconds since the clip started (it loops).</param>
/// <param name="Playing">Whether it moves.</param>
public sealed record PreviewState(string Clip, float Time, bool Playing);

/// <summary>One object's player and what it plays on.</summary>
internal sealed class Animated(string model, Skeleton skeleton, Clips clips)
{
    /// <summary>The model file (the Mesh's <c>model</c>) the player was made for.</summary>
    public string Model { get; } = model;

    public Skeleton Skeleton { get; } = skeleton;

    public Clips Clips { get; } = clips;

    public Player Player { get; } = new();

    /// <summary>Outside play mode: whether its time stands still (a preview paused or scrubbed).</summary>
    public bool Paused { get; set; }

    /// <summary>Each node's matrix into the model's space (before the import scale and axes), as the player poses it now.</summary>
    public Matrix4x4[] World() => Skeleton.World(Player.Pose(Skeleton, Clips));
}

/// <summary>The session's animated objects and their poses.</summary>
internal sealed class SessionAnimation
{
    public AssetCache Cache { get; } = new();

    public Dictionary<StableId, Animated> Objects { get; } = [];

    /// <summary>Each animated object's nodes, as the views draw them.</summary>
    public Dictionary<StableId, Matrix4x4[]> Poses { get; } = [];

    /// <summary>Drops every player and pose (play mode starts or stops).</summary>
    public void Clear()
    {
        Objects.Clear();
        Poses.Clear();
    }
}

// Skeletal animation in the session (docs/animation.md): the poses the views draw, and the editor's previews.
public sealed partial class Session
{
    internal SessionAnimation Animation { get; } = new();

    /// <summary>The nodes of each object an animation player poses (each node's matrix into its model, before the import scale and axes), for the views to draw.</summary>
    public IReadOnlyDictionary<StableId, Matrix4x4[]> AnimationPoses => Animation.Poses;

    /// <summary>The preview of object <paramref name="id"/> outside play mode: the clip its player shows, how far in, and whether it moves.</summary>
    public PreviewState? AnimationPreview(StableId id)
    {
        if (IsPlaying || !Animation.Objects.TryGetValue(id, out var a))
        {
            return null;
        }
        return a.Player.LayerOn("")?.Current is ({ } motion, var time) && motion is Motion.Clip clip ? new PreviewState(clip.Name, time, !a.Paused) : null;
    }

    /// <summary>Whether an animation preview moves outside play mode (the editor then keeps calling <see cref="Animate"/>).</summary>
    public bool AnimationPreviewing => !IsPlaying && Animation.Objects.Values.Any(a => !a.Paused && a.Player.IsPlaying);

    /// <summary>Moves the editor's animation previews on by <paramref name="dt"/> seconds (outside play mode; play mode moves its own players as it steps).</summary>
    public void Animate(float dt)
    {
        if (IsPlaying)
        {
            return;
        }
        foreach (var (id, a) in Animation.Objects)
        {
            if (a.Paused)
            {
                continue;
            }
            a.Player.Advance(a.Skeleton, a.Clips, dt);
            Animation.Poses[id] = a.World();
        }
    }
}

/// <summary>
/// The <c>animation.*</c> commands: a player for an animated object, made by the first command that
/// names it, whose clips move only while the editor calls <see cref="Session.Animate"/>; the scene
/// file never changes.
/// </summary>
/// <remarks>
/// Outside play mode a player is made with the clips of the model and the moments the Model
/// Animation marks, playing nothing. In play mode (<c>AnimationPlay.cs</c>) each object with a Model
/// Animation gets a player when play starts or it comes, playing its clip or running its animation
/// states (with keyframe animations as clips); root motion moves the object, each step moves every
/// player, clip events reach the object's scripts, and scripts play clips and read joints.
/// </remarks>
internal static partial class AnimationCommands
{
    /// <summary>Longest clip or joint name, in bytes.</summary>
    private const int MaxName = 256;

    /// <summary>Most clips one mix holds.</summary>
    private const int MaxMix = 16;

    private const float DefaultFade = 0.25f;

    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "animation.get", Label = "Read Animation",
            Params = "{id}: the object's animation: {clips: [{name, duration}], joints: [names], layers: [{joint, motion, time, fading}], parameters: {name: value}, playing, state (the animation state it is in, or null), states (its states' names, or null)}",
            Run = Get,
        },
        new()
        {
            Id = "animation.play", Label = "Play Clip",
            Params = "{id, clip? | mix?: [{clip, at}], by?: number or parameter name, layer?: joint, fade?: seconds, loop?: true, speed?: 1}: plays a clip (or a mix of clips by where `by` lies among their points) on the object, fading from what played over `fade` (0 to 60, default 0.25); `layer` plays it on that joint and the ones under it only, over the rest",
            Run = Play,
        },
        new() { Id = "animation.stop", Label = "Stop Clip", Params = "{id, layer?: joint, fade?: seconds}: the whole skeleton fades to rest; a layer fades out and goes", Run = StopClip },
        new()
        {
            Id = "animation.parameter.set", Label = "Set Animation Parameter",
            Params = "{id, name, value: number | true/false}: a parameter mixes and animation states read", Run = ParameterSet,
        },
        new() { Id = "animation.trigger", Label = "Trigger Animation Parameter", Params = "{id, name}: sets a trigger, which the first transition that uses it uses up", Run = Trigger },
        new()
        {
            Id = "animation.preview", Label = "Preview Clip",
            Params = "{id, clip, time?: seconds, playing?: true/false}: outside play mode, shows the clip on the object's model in the views (the Inspector's preview), looping: from `time` when given (scrubbing), moving while `playing`; the scene file does not change",
            Enabled = OutsidePlay, Run = Preview,
        },
        new()
        {
            Id = "animation.preview.stop", Label = "Stop Preview", Params = "{id}: ends the object's preview; the views show its Model Animation again",
            Enabled = OutsidePlay, Run = PreviewStop,
        },
        new()
        {
            Id = "animation.joint.get", Label = "Read Joint",
            Params = "{id, joint}: where the joint of the object's model is in the world now, as its animation poses it (at rest when nothing plays): {position, rotation (degrees), scale}",
            Run = JointGet,
        },
        new()
        {
            Id = "animation.state.set", Label = "Go to Animation State",
            Params = "{id, state, fade?: seconds}: an object running animation states goes to that state now, fading over `fade` (default 0.25)",
            Run = StateSet,
        },
    ];

    private static string? OutsidePlay(Session session) => session.IsPlaying ? "previews are for outside play mode" : null;

    /// <summary>The skeleton of object <paramref name="id"/>'s model, read through the session's confinement.</summary>
    private static (string Model, Skeleton Skeleton) LoadSkeleton(Session session, StableId id)
    {
        if (!session.Scene.TryGet<Mesh>(id, out var mesh))
        {
            throw Stop.Failed($"object {id} has no Mesh, so no model to animate");
        }
        if (mesh.Model.Length == 0)
        {
            throw Stop.Failed($"object {id}'s Mesh has no model to animate");
        }
        var model = mesh.Model;
        var path = session.Allowed(Path.Combine(session.ScriptDir, model), Access.Read);
        var data = session.Animation.Cache.Model(path);
        if (data.IsError)
        {
            throw Stop.Failed($"{model}: {data.Error}");
        }
        var skeleton = data.Value.Skeleton ?? throw Stop.Failed($"{model} has no skeleton or animations");
        return (model, skeleton);
    }

    /// <summary>
    /// A new player for object <paramref name="id"/>, with the model's clips and the moments its
    /// Model Animation marks in them. Outside play mode it plays nothing until a command says so; in
    /// play mode it plays what the Model Animation sets (<see cref="Autoplay"/>).
    /// </summary>
    private static Animated NewAnimated(Session session, StableId id, string model, Skeleton skeleton)
    {
        var marks = session.Scene.TryGet<ModelAnimation>(id, out var settings)
            ? settings.Events.Select(m => (m.Clip, new ClipEvent(m.Time, m.Event)))
            : [];
        var made = new Animated(model, skeleton, Clips.OfModel(skeleton, marks));
        if (session.IsPlaying && settings is not null)
        {
            Autoplay(session, made, settings);
        }
        return made;
    }

    /// <summary>Object <paramref name="id"/>'s player, made when it has none (or its model changed).</summary>
    private static Animated AnimatedOf(Session session, StableId id)
    {
        if (!session.Scene.Contains(id))
        {
            throw Stop.Failed($"there is no object {id}");
        }
        var current = session.Scene.TryGet<Mesh>(id, out var mesh) ? mesh.Model : null;
        if (!session.Animation.Objects.TryGetValue(id, out var a) || a.Model != current)
        {
            var (model, skeleton) = LoadSkeleton(session, id);
            a = NewAnimated(session, id, model, skeleton);
            session.Animation.Objects[id] = a;
        }
        return a;
    }

    /// <summary>Keeps the player's pose for the views and gives what <c>animation.get</c> gives.</summary>
    private static JsonObject Posed(Session session, StableId id, Animated a)
    {
        session.Animation.Poses[id] = a.World();
        return Describe(a);
    }

    private static JsonValue MotionJson(Motion motion) => motion switch
    {
        Motion.Clip c => J.Obj(("clip", J.S(c.Name)), ("speed", JsonNumber.Of(c.Speed)), ("loop", J.B(c.Looping))),
        Motion.Mix m => J.Obj(
            ("mix", new JsonArray(m.Items.Select(i => (JsonValue)J.Obj(("motion", MotionJson(i.Motion)), ("at", JsonNumber.Of(i.At)))))),
            ("by", NumberJson(m.By))),
        Motion.Layer l => J.Obj(("base", MotionJson(l.Base)), ("top", MotionJson(l.Top)), ("joint", J.S(l.Joint)), ("weight", NumberJson(l.Weight))),
        _ => J.S("rest"),
    };

    private static JsonValue NumberJson(Number number) => number.Parameter is { } name ? J.S(name) : JsonNumber.Of(number.Value);

    private static JsonValue ParameterJson(Parameter value) => value.Kind switch
    {
        ParameterKind.Number => JsonNumber.Of(value.AsNumber()),
        ParameterKind.Flag => J.B(value.AsFlag()),
        _ => J.Obj(("trigger", J.B(value.AsFlag()))),
    };

    /// <summary>What an object's animation is now, as <c>animation.get</c> gives it.</summary>
    internal static JsonObject Describe(Animated a)
    {
        var clips = a.Clips.Names.Select(n => (JsonValue)J.Obj(("name", J.S(n)), ("duration", JsonNumber.Of(a.Clips.Get(n)?.Duration ?? 0f))));
        var layers = a.Player.Layers.Select(l =>
        {
            var (motion, time) = l.Current is { } current ? (MotionJson(current.Motion), current.Time) : (J.S("rest"), 0f);
            return (JsonValue)J.Obj(("joint", J.S(l.Joint)), ("motion", motion), ("time", JsonNumber.Of(time)), ("fading", J.B(l.Fading)));
        });
        var parameters = new JsonObject();
        foreach (var (name, value) in a.Player.Parameters)
        {
            parameters.Add(name, ParameterJson(value));
        }
        var states = a.Player.Machine is { } machine ? J.Strings(machine.States.Select(s => s.Name)) : JsonValue.Null;
        return J.Obj(
            ("clips", new JsonArray(clips)),
            ("joints", J.Strings(a.Skeleton.Names)),
            ("layers", new JsonArray(layers)),
            ("parameters", parameters),
            ("playing", J.B(a.Player.IsPlaying)),
            ("state", J.S(a.Player.State)),
            ("states", states));
    }

    private static JsonObject Get(Session session, JsonValue p)
    {
        var id = Params.Of(p, "id").Required("id", JsonRead.Id);
        return Describe(AnimatedOf(session, id));
    }

    private static int Bytes(string text) => Encoding.UTF8.GetByteCount(text);

    private static void CheckName(string what, string name)
    {
        if (Bytes(name) > MaxName)
        {
            throw Stop.BadParams($"{what} is longer than {MaxName} bytes");
        }
    }

    private static void CheckFade(float fade)
    {
        if (!(float.IsFinite(fade) && fade >= 0f && fade <= Player.MaxFade))
        {
            throw Stop.BadParams(JsonRead.Invariant($"fade must be from 0 to {Player.MaxFade} seconds"));
        }
    }

    /// <summary>A number a motion reads: a number as it is, or text naming a parameter.</summary>
    private static Number NumberParam(JsonValue value)
    {
        switch (value)
        {
            case JsonNumber n when (float)n.ToDouble() is var v && float.IsFinite(v):
                return Number.Of(v);
            case JsonString { Value: { Length: > 0 } name } when Bytes(name) <= MaxName:
                return Number.Named(name);
            default:
                throw Stop.BadParams("by must be a number or a parameter's name");
        }
    }

    private static (string Clip, float At) MixItemOf(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct MixItem", true, "clip", "at");
        return (f.Required("clip", JsonRead.Str), f.Required("at", JsonRead.F32));
    }

    private static JsonObject Play(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "clip", "mix", "by", "layer", "fade", "loop", "speed");
        var id = f.Required("id", JsonRead.Id);
        var clipName = f.Nullable("clip", JsonRead.Str);
        var mix = f.Nullable("mix", v => JsonRead.List(v, MixItemOf));
        var by = f.Nullable("by", v => v);
        var layer = f.Optional("layer", JsonRead.Str, "");
        var fade = f.Optional("fade", JsonRead.F32, DefaultFade);
        var looping = f.Optional("loop", JsonRead.Bool, true);
        var speed = f.Optional("speed", JsonRead.F32, 1f);
        CheckName("layer", layer);
        CheckFade(fade);
        if (!(float.IsFinite(speed) && speed >= 0f && speed <= 10f))
        {
            throw Stop.BadParams("speed must be from 0 to 10");
        }
        Motion Clip(string name) => new Motion.Clip(name, speed, looping);
        Motion motion;
        switch (clipName, mix)
        {
            case ({ } name, null):
                CheckName("clip", name);
                motion = Clip(name);
                break;
            case (null, { } items):
                if (items.Count is 0 or > MaxMix)
                {
                    throw Stop.BadParams($"a mix holds 1 to {MaxMix} clips");
                }
                var mixed = new List<MixItem>(items.Count);
                foreach (var item in items)
                {
                    CheckName("clip", item.Clip);
                    if (!float.IsFinite(item.At))
                    {
                        throw Stop.BadParams("each clip's point must be a number");
                    }
                    mixed.Add(new MixItem(Clip(item.Clip), item.At));
                }
                motion = new Motion.Mix(mixed, by is null ? Number.Of(0f) : NumberParam(by));
                break;
            default:
                throw Stop.BadParams("give a clip or a mix");
        }
        var a = AnimatedOf(session, id);
        var problems = motion.Problems(a.Skeleton, a.Clips);
        if (layer.Length > 0 && a.Skeleton.Mask(layer) is null)
        {
            throw Stop.Failed($"there is no joint named \"{layer}\"; the model has {string.Join(", ", a.Skeleton.Names)}");
        }
        if (problems.Count > 0)
        {
            throw Stop.Failed(problems[0]);
        }
        a.Player.Play(layer, motion, fade).OrFail();
        a.Paused = false;
        return Posed(session, id, a);
    }

    private static JsonObject StopClip(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "layer", "fade");
        var id = f.Required("id", JsonRead.Id);
        var layer = f.Optional("layer", JsonRead.Str, "");
        var fade = f.Optional("fade", JsonRead.F32, DefaultFade);
        CheckName("layer", layer);
        CheckFade(fade);
        var a = AnimatedOf(session, id);
        if (layer.Length > 0 && a.Player.LayerOn(layer) is null)
        {
            throw Stop.Failed($"no layer plays on \"{layer}\"");
        }
        a.Player.Stop(layer, fade);
        return Posed(session, id, a);
    }

    private static void CheckParameterName(string name)
    {
        if (name.Length == 0 || Bytes(name) > MaxName)
        {
            throw Stop.BadParams($"a parameter's name is 1 to {MaxName} bytes");
        }
    }

    private static JsonObject ParameterSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "name", "value");
        var id = f.Required("id", JsonRead.Id);
        var name = f.Required("name", JsonRead.Str);
        var given = f.Required("value", v => v);
        CheckParameterName(name);
        var value = given switch
        {
            JsonBool b => Parameter.Flag(b.Value),
            JsonNumber n when n.ToDouble() is var v && double.IsFinite(v) && Math.Abs(v) <= 1.0e9 => Parameter.Number((float)v),
            JsonNumber => throw Stop.BadParams("value must be a number of at most a billion, or true/false"),
            _ => throw Stop.BadParams("value must be a number or true/false"),
        };
        return SetParameter(session, id, name, value);
    }

    private static JsonObject SetParameter(Session session, StableId id, string name, Parameter value)
    {
        var a = AnimatedOf(session, id);
        a.Player.SetParameter(name, value).OrFail();
        return Posed(session, id, a);
    }

    private static JsonObject Trigger(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "name");
        var id = f.Required("id", JsonRead.Id);
        var name = f.Required("name", JsonRead.Str);
        CheckParameterName(name);
        return SetParameter(session, id, name, Parameter.Trigger(true));
    }

    private static DMat4 ToDMat4(Matrix4x4 m) =>
        new(new(m.M11, m.M12, m.M13, m.M14), new(m.M21, m.M22, m.M23, m.M24), new(m.M31, m.M32, m.M33, m.M34), new(m.M41, m.M42, m.M43, m.M44));

    /// <summary>Where joint <paramref name="joint"/> of object <paramref name="id"/>'s model is in the world now: by its player when it has one, else at rest.</summary>
    internal static DMat4 JointWorld(Session session, StableId id, string joint)
    {
        var world = session.Scene.WorldMatrix(id) ?? throw Stop.Failed($"there is no object {id}");
        Skeleton skeleton;
        Matrix4x4[] nodes;
        if (session.Animation.Objects.TryGetValue(id, out var a) && session.Scene.TryGet<Mesh>(id, out var mesh) && mesh.Model == a.Model)
        {
            (skeleton, nodes) = (a.Skeleton, a.World());
        }
        else
        {
            skeleton = LoadSkeleton(session, id).Skeleton;
            nodes = skeleton.PoseAt(null, 0f);
        }
        var node = skeleton.NodeNamed(joint)
            ?? throw Stop.Failed($"there is no joint named \"{joint}\"; the model has {string.Join(", ", skeleton.Names)}");
        // The node's matrix into the model, then the import scale and axes.
        var inModel = (node < nodes.Length ? nodes[node] : Matrix4x4.Identity) * skeleton.Root;
        return world * ToDMat4(inModel);
    }

    private static JsonObject JointGet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "joint");
        var id = f.Required("id", JsonRead.Id);
        var joint = f.Required("joint", JsonRead.Str);
        CheckName("joint", joint);
        var t = Transform.FromMatrix(JointWorld(session, id, joint));
        return J.Obj(
            ("position", new JsonArray([JsonNumber.Of(t.Position.X), JsonNumber.Of(t.Position.Y), JsonNumber.Of(t.Position.Z)])),
            ("rotation", new JsonArray([JsonNumber.Of(t.Rotation.X), JsonNumber.Of(t.Rotation.Y), JsonNumber.Of(t.Rotation.Z)])),
            ("scale", new JsonArray([JsonNumber.Of(t.Scale.X), JsonNumber.Of(t.Scale.Y), JsonNumber.Of(t.Scale.Z)])));
    }

    private static JsonObject StateSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "state", "fade");
        var id = f.Required("id", JsonRead.Id);
        var state = f.Required("state", JsonRead.Str);
        var fade = f.Optional("fade", JsonRead.F32, DefaultFade);
        CheckName("state", state);
        CheckFade(fade);
        var a = AnimatedOf(session, id);
        a.Player.GoTo(state, fade).OrFail();
        a.Paused = false;
        return Posed(session, id, a);
    }

    private static JsonObject Preview(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "clip", "time", "playing");
        var id = f.Required("id", JsonRead.Id);
        var clip = f.Required("clip", JsonRead.Str);
        var time = f.NullableValue("time", JsonRead.F32);
        var playing = f.NullableValue("playing", JsonRead.Bool);
        CheckName("clip", clip);
        if (time is { } t && !(float.IsFinite(t) && t >= 0f && t <= 1.0e6f))
        {
            throw Stop.BadParams("time must be from 0 to a million seconds");
        }
        var a = AnimatedOf(session, id);
        var shown = a.Player.LayerOn("")?.Current is ({ } current, _) && current is Motion.Clip c && c.Name == clip;
        if (!shown)
        {
            var motion = Motion.OfClip(clip);
            if (motion.Problems(a.Skeleton, a.Clips) is [var problem, ..])
            {
                throw Stop.Failed(problem);
            }
            a.Player.Play("", motion, 0f).OrFail();
        }
        if (time is { } at)
        {
            a.Player.Seek("", at);
        }
        if (playing is { } moving)
        {
            a.Paused = !moving;
        }
        return Posed(session, id, a);
    }

    private static JsonObject PreviewStop(Session session, JsonValue p)
    {
        var id = Params.Of(p, "id").Required("id", JsonRead.Id);
        session.Animation.Objects.Remove(id);
        session.Animation.Poses.Remove(id);
        return J.Obj(("stopped", J.Id(id)));
    }
}
