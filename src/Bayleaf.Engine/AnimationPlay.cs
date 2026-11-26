using System.Numerics;

using Bayleaf.Animation;
using Bayleaf.Assets;
using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;
using Bayleaf.Script;

using ModelAnimation = Bayleaf.Scene.Components.ModelAnimation;
using Motion = Bayleaf.Animation.Motion;
using TimelineModel = Bayleaf.Scene.Timeline;
using Transform = Bayleaf.Scene.Components.Transform;

namespace Bayleaf.Engine;

// Skeletal animation in play mode: the players play mode makes and steps, root motion, clip events
// for scripts, and what scripts ask of an object's animation.
internal static partial class AnimationCommands
{
    /// <summary>Samples per second a keyframe animation is taken at when animation states play it as a clip, so every ease reads as the timeline shows it.</summary>
    private const float TimelineRate = 60f;

    /// <summary>What a new player plays in play mode, as its Model Animation sets it: root motion, its animation states, or its clip.</summary>
    private static void Autoplay(Session session, Animated a, ModelAnimation settings)
    {
        var player = a.Player;
        // Root motion moves the object only in play mode, where the scene is a copy.
        if (settings.RootMotion)
        {
            player.Root = settings.RootJoint.Length == 0 ? Motion.DefaultRoot(a.Skeleton) : a.Skeleton.NodeNamed(settings.RootJoint);
            if (player.Root is null)
            {
                session.Log(Level.Error, $"Root motion: the model has no joint named \"{settings.RootJoint}\"");
            }
        }
        if (settings.States.Length > 0)
        {
            var machine = LoadStates(session, settings.States);
            if (machine.IsError)
            {
                session.Log(Level.Error, $"{settings.States}: {machine.Error}; the object plays no animation states");
                return;
            }
            // Keyframe animations the states name are clips too.
            var named = new SortedSet<string>(StringComparer.Ordinal);
            foreach (var state in machine.Value.States)
            {
                ClipNames(state.Motion, named);
            }
            foreach (var path in named.Where(n => Text.AsciiLower(n).EndsWith("." + TimelineModel.Extension, StringComparison.Ordinal)))
            {
                var clip = TimelineClip(session, path);
                if (clip.IsOk)
                {
                    a.Clips.Insert(path, clip.Value);
                }
                else
                {
                    session.Log(Level.Error, $"{settings.States}: the clip {path} does not read: {clip.Error}");
                }
            }
            foreach (var problem in machine.Value.States.SelectMany(s => s.Motion.Problems(a.Skeleton, a.Clips)))
            {
                session.Log(Level.Error, $"{settings.States}: {problem}");
            }
            player.RunStates(machine.Value);
            return;
        }
        if (settings.Playing && a.Skeleton.Animation(settings.Animation) is { } index && index < a.Skeleton.Animations.Count)
        {
            player.Play("", new Motion.Clip(a.Skeleton.Animations[index].Name, settings.Speed, settings.Looping), 0f);
        }
    }

    /// <summary>Every clip a motion names: its own, and its mix's and layers' clips.</summary>
    private static void ClipNames(Motion motion, SortedSet<string> output)
    {
        switch (motion)
        {
            case Motion.Clip c:
                output.Add(c.Name);
                break;
            case Motion.Mix m:
                foreach (var item in m.Items)
                {
                    ClipNames(item.Motion, output);
                }
                break;
            case Motion.Layer l:
                ClipNames(l.Base, output);
                ClipNames(l.Top, output);
                break;
        }
    }

    /// <summary>Reads and compiles the animation states at <paramref name="path"/> (relative to the project folder).</summary>
    private static Result<Machine, string> LoadStates(Session session, string path)
    {
        try
        {
            var full = session.Allowed(Path.Combine(session.ScriptDir, path), Access.Read);
            var text = Files.ReadText(full);
            if (text.IsError)
            {
                return Result.Fail($"cannot be read: {text.Error}");
            }
            var graph = Graph.Graph.FromJson(text.Value, AnimationStates.Catalog);
            if (graph.IsError)
            {
                return Result.Fail(graph.Error.Message);
            }
            var machine = AnimationStates.Compile(graph.Value);
            return machine.IsOk ? machine.Value : Result.Fail(Graph.Problem.Summary(machine.Error));
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error.Message);
        }
    }

    /// <summary>
    /// A keyframe animation (<c>.btimeline</c>) as a clip for a model: the position, rotation and scale
    /// tracks of its parts, each posing the model's node named as the part is in the timeline's prefab.
    /// </summary>
    private static Result<NamedClip, string> TimelineClip(Session session, string path)
    {
        try
        {
            var full = session.Allowed(Path.Combine(session.ScriptDir, path), Access.Read);
            var text = Files.ReadText(full);
            if (text.IsError)
            {
                return Result.Fail($"cannot read it: {text.Error}");
            }
            var timeline = TimelineModel.FromJson(text.Value);
            if (timeline.IsError)
            {
                return timeline.AsFailure();
            }
            var objects = PrefabCommands.ReadPrefab(session, timeline.Value.Prefab);
            var length = Math.Max(timeline.Value.Length, 1e-3f);
            var steps = (int)Math.Clamp(MathF.Ceiling(length * TimelineRate), 1f, 100_000f);
            var times = Enumerable.Range(0, steps + 1).Select(i => Math.Min(i / TimelineRate, length)).ToList();
            var tracks = new List<NamedTrack>();
            foreach (var part in timeline.Value.Parts)
            {
                if (objects.Find(o => o.Id == part.Id)?.Components.GetValueOrDefault("name") is not JsonString { Value: var name })
                {
                    continue;
                }
                foreach (var track in part.Tracks)
                {
                    if (track.Target is not Target.Field { Component: "transform" } field)
                    {
                        continue;
                    }
                    ChannelProperty? property = field.Name switch
                    {
                        "position" => ChannelProperty.Translation,
                        "rotation" => ChannelProperty.Rotation,
                        "scale" => ChannelProperty.Scale,
                        _ => null,
                    };
                    if (property is not { } kind)
                    {
                        continue;
                    }
                    var values = new List<Vector4>(times.Count);
                    foreach (var t in times)
                    {
                        if (track.ValueAt(t) is not JsonArray { Count: >= 3 } items || items.Take(3).Any(i => i is not JsonNumber))
                        {
                            values.Clear();
                            break;
                        }
                        var (x, y, z) = ((float)((JsonNumber)items[0]).ToDouble(), (float)((JsonNumber)items[1]).ToDouble(), (float)((JsonNumber)items[2]).ToDouble());
                        if (kind == ChannelProperty.Rotation)
                        {
                            // Degrees about X, then Y, then Z, as a Transform turns.
                            var q = DQuat.FromEulerXyz(x * Math.PI / 180, y * Math.PI / 180, z * Math.PI / 180);
                            values.Add(new Vector4((float)q.X, (float)q.Y, (float)q.Z, (float)q.W));
                        }
                        else
                        {
                            values.Add(new Vector4(x, y, z, 0f));
                        }
                    }
                    if (values.Count == times.Count)
                    {
                        tracks.Add(new NamedTrack(name, kind, false, times, values));
                    }
                }
            }
            return new NamedClip(length, tracks, []);
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error.Message);
        }
    }

    /// <summary>Play mode started (or a scene took the playing one's place): every object with a Model Animation gets a player.</summary>
    public static void StartPlay(Session session)
    {
        session.Animation.Clear();
        Sync(session);
    }

    /// <summary>Gives each object with a Model Animation and a model a player (spawned ones too), and drops the players of objects that went.</summary>
    private static void Sync(Session session)
    {
        var scene = session.Scene;
        foreach (var gone in session.Animation.Objects.Keys.Where(id => !scene.Contains(id)).ToList())
        {
            session.Animation.Objects.Remove(gone);
            session.Animation.Poses.Remove(gone);
        }
        foreach (var id in ScriptHost.Order(scene))
        {
            if (!scene.TryGet<ModelAnimation>(id, out _) || session.Animation.Objects.ContainsKey(id))
            {
                continue;
            }
            // A model that does not load or has no skeleton is drawn as it is.
            try
            {
                var (model, skeleton) = LoadSkeleton(session, id);
                session.Animation.Objects[id] = NewAnimated(session, id, model, skeleton);
            }
            catch (CommandException)
            {
            }
        }
    }

    /// <summary>One play step: every player moves on and its pose is kept for the views; root motion moves its object, and the clip events it passed go to the object's scripts.</summary>
    public static void Step(Session session, float dt)
    {
        if (session.Play is not { } play)
        {
            return;
        }
        if (play.Changed("animation", session.Scene))
        {
            Sync(session);
        }
        if (session.Animation.Objects.Count == 0)
        {
            return;
        }
        var events = new List<(StableId, string)>();
        foreach (var (id, a) in session.Animation.Objects.ToList())
        {
            var step = a.Player.Advance(a.Skeleton, a.Clips, dt);
            session.Animation.Poses[id] = a.World();
            events.AddRange(step.Events.Select(e => (id, e)));
            if (step.Root.LengthSquared() > 0f)
            {
                MoveByRoot(session, id, step.Root);
            }
        }
        play.Host.Animation = ScriptView(session);
        if (events.Count == 0)
        {
            return;
        }
        play.Host.TriggerEvents(session.Scene, events, play.Time);
        PlayLoop.AfterScripts(session);
    }

    /// <summary>What scripts read of the animated objects after this step: each one's state and its joints' places in the world.</summary>
    private static Dictionary<StableId, AnimatedView> ScriptView(Session session)
    {
        var view = new Dictionary<StableId, AnimatedView>();
        foreach (var (id, a) in session.Animation.Objects)
        {
            if (session.Scene.WorldMatrix(id) is not { } world || !session.Animation.Poses.TryGetValue(id, out var nodes))
            {
                continue;
            }
            var joints = nodes.Select(m =>
            {
                var t = Transform.FromMatrix(world * ToDMat4(m * a.Skeleton.Root));
                return (t.Position, t.Rotation, t.Scale);
            }).ToList();
            view[id] = new AnimatedView(a.Player.State, a.Skeleton.Names, joints);
        }
        return view;
    }

    /// <summary>Does what a script asked of an object's animation; what cannot be done goes to the console.</summary>
    public static void ApplyScript(Session session, StableId by, StableId obj, AnimationAction action)
    {
        try
        {
            var a = AnimatedOf(session, obj);
            switch (action)
            {
                case AnimationAction.Play play:
                    var motion = new Motion.Clip(play.Clip, 1f, play.Looping);
                    if (motion.Problems(a.Skeleton, a.Clips) is [var problem, ..])
                    {
                        throw Stop.Failed(problem);
                    }
                    a.Player.Play("", motion, play.Fade).OrFail();
                    break;
                case AnimationAction.Rest rest:
                    a.Player.Stop("", rest.Fade);
                    break;
                case AnimationAction.Number n:
                    a.Player.SetParameter(n.Name, Parameter.Number(n.Value)).OrFail();
                    break;
                case AnimationAction.Flag f:
                    a.Player.SetParameter(f.Name, Parameter.Flag(f.Value)).OrFail();
                    break;
                case AnimationAction.Trigger t:
                    a.Player.SetParameter(t.Name, Parameter.Trigger(true)).OrFail();
                    break;
            }
        }
        catch (CommandException e)
        {
            session.Log(Level.Error, $"[{session.Scene.NameOf(by) ?? "Object"}] cannot change the animation of {session.Scene.NameOf(obj) ?? "Object"}: {e.Error.Message}");
        }
    }

    /// <summary>Moves object <paramref name="id"/> by <paramref name="delta"/>, a move in its model: through the object's own turn and scale into the world, then into its parent's space.</summary>
    private static void MoveByRoot(Session session, StableId id, Vector3 delta)
    {
        if (session.Scene.WorldMatrix(id) is not { } world || !session.Scene.TryGet<Transform>(id, out var transform))
        {
            return;
        }
        var inWorld = world.TransformVector3(new Double3(delta.X, delta.Y, delta.Z));
        var inParent = session.Scene.Parent(id) is { } parent && session.Scene.WorldMatrix(parent) is { } p ? p.Inverse().TransformVector3(inWorld) : inWorld;
        var moved = transform with { Position = transform.Position + inParent };
        if (moved.Position.IsFinite)
        {
            session.Scene.Set(id, moved);
        }
    }
}
