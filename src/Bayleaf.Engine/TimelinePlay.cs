using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using TimelineModel = Bayleaf.Scene.Timeline;

namespace Bayleaf.Engine;

/// <summary>The Keyframe Animations of the playing scene, and the timeline files read this play.</summary>
internal sealed class TimelinePlayers
{
    public sealed class Player(StableId root, TimelineModel timeline, Binding binding, bool playing)
    {
        public StableId Root { get; } = root;

        public TimelineModel Timeline { get; } = timeline;

        public Binding Binding { get; } = binding;

        /// <summary>Seconds played since it started.</summary>
        public double Time { get; set; }

        public bool Playing { get; set; } = playing;

        /// <summary>Whether its problems were already reported.</summary>
        public bool Told { get; set; }
    }

    public List<Player> Players { get; } = [];

    /// <summary>Timeline files read this play, by path: the timeline, or why it could not be read.</summary>
    public Dictionary<string, Result<TimelineModel, string>> Files { get; } = new(StringComparer.Ordinal);

    public void Clear()
    {
        Players.Clear();
        Files.Clear();
    }
}

/// <summary>
/// Keyframe animations in play mode: each object's Keyframe Animation component plays its
/// <c>.btimeline</c> file on the parts of the prefab the object is the root of; <c>timeline.play</c> and
/// <c>timeline.stop</c> (and scripts) start and stop it.
/// </summary>
internal static class TimelinePlay
{
    public static IEnumerable<CommandSpec> All() =>
    [
        new() { Id = "timeline.play", Label = "Play Keyframe Animation", Params = "{id}: in play mode, plays the object's Keyframe Animation from its start", Enabled = PlayCommands.Playing, Run = (s, p) => Command(s, p, true) },
        new() { Id = "timeline.stop", Label = "Stop Keyframe Animation", Params = "{id}: in play mode, stops the object's Keyframe Animation where it is", Enabled = PlayCommands.Playing, Run = (s, p) => Command(s, p, false) },
    ];

    private static JsonObject Command(Session session, JsonValue p, bool play)
    {
        var id = Params.Of(p, "id").Required("id", JsonRead.Id);
        if (SetPlaying(session, id, play) is { } problem)
        {
            throw Stop.Failed(problem);
        }
        return [];
    }

    /// <summary>Plays the Keyframe Animation of <paramref name="obj"/> from its start, or stops it where it is; why it cannot, or null.</summary>
    public static string? SetPlaying(Session session, StableId obj, bool play)
    {
        if (session.Play is not { } state)
        {
            return "play mode is not running";
        }
        if (!state.Timelines.Players.Any(t => t.Root == obj) && StartUnder(session, [obj]).FirstOrDefault() is { } problem)
        {
            // Added since it started, or never started: it is read now.
            return problem;
        }
        if (state.Timelines.Players.Find(t => t.Root == obj) is not { } player)
        {
            return "it has no Keyframe Animation with an animation file";
        }
        if (play)
        {
            player.Time = 0;
        }
        player.Playing = play;
        return null;
    }

    /// <summary>Reads the Keyframe Animations of <paramref name="roots"/> and everything under them (the whole scene for null) and starts those that play on start; what could not be read.</summary>
    public static List<string> StartUnder(Session session, IReadOnlyList<StableId>? roots)
    {
        var problems = new List<string>();
        if (session.Play is not { } play)
        {
            return problems;
        }
        var scene = session.Scene;
        foreach (var id in Script.ScriptHost.Order(scene))
        {
            if (roots is not null && !roots.Any(r => r == id || scene.IsInside(id, r)))
            {
                continue;
            }
            if (!scene.TryGet<KeyframeAnimation>(id, out var component) || component.Animation.Length == 0)
            {
                continue;
            }
            var timeline = Read(session, play, component.Animation);
            if (timeline.IsError)
            {
                problems.Add($"[{scene.NameOf(id) ?? "Object"}] cannot play {component.Animation}: {timeline.Error}");
                continue;
            }
            var binding = Bind(session, id, timeline.Value);
            play.Timelines.Players.RemoveAll(p => p.Root == id);
            play.Timelines.Players.Add(new TimelinePlayers.Player(id, timeline.Value, binding, component.PlayOnStart));
        }
        return problems;
    }

    /// <summary>The timeline at <paramref name="relative"/> (to the project folder), read once per play.</summary>
    private static Result<TimelineModel, string> Read(Session session, PlayState play, string relative)
    {
        if (play.Timelines.Files.TryGetValue(relative, out var known))
        {
            return known;
        }
        Result<TimelineModel, string> read;
        try
        {
            var path = session.Allowed(Path.Combine(session.ScriptDir, relative), Access.Read);
            var text = Core.Files.ReadText(path);
            read = text.IsOk ? TimelineModel.FromJson(text.Value) : Result.Fail($"cannot read it: {text.Error}");
        }
        catch (CommandException e)
        {
            read = Result.Fail(e.Error.Message);
        }
        play.Timelines.Files[relative] = read;
        return read;
    }

    /// <summary>The objects of <paramref name="root"/>'s copy that stand for the timeline's parts: by hierarchy order when it is a copy of the timeline's prefab, else by the parts' own ids under it.</summary>
    private static Binding Bind(Session session, StableId root, TimelineModel timeline)
    {
        if (session.Scene.TryGet<Prefab>(root, out var prefab) && prefab.Source == timeline.Prefab)
        {
            try
            {
                var ids = PrefabCommands.ReadPrefab(session, timeline.Prefab).Select(o => o.Id).ToList();
                return Binding.ByOrder(session.Scene, root, ids, timeline);
            }
            catch (CommandException)
            {
            }
        }
        var binding = Binding.SameIds(session.Scene, timeline);
        foreach (var (part, obj) in binding.Objects.ToList())
        {
            if (obj != root && !session.Scene.IsInside(obj, root))
            {
                binding.Objects.Remove(part);
            }
        }
        return binding;
    }

    /// <summary>Moves every playing Keyframe Animation on by <paramref name="dt"/> seconds and poses its parts.</summary>
    public static void Step(Session session, float dt)
    {
        if (session.Play is not { } play)
        {
            return;
        }
        // Objects that went take their animation with them.
        play.Timelines.Players.RemoveAll(p => !session.Scene.Contains(p.Root));
        var reports = new List<string>();
        foreach (var player in play.Timelines.Players.Where(p => p.Playing))
        {
            var speed = session.Scene.TryGet<KeyframeAnimation>(player.Root, out var k) ? k.Speed : 1f;
            player.Time += dt * (double)speed;
            var samples = player.Timeline.SampleAt(player.Timeline.LocalTime(player.Time));
            var problems = TimelineModel.Apply(session.Scene, player.Binding, samples);
            if (player.Timeline.Finished(player.Time))
            {
                player.Playing = false;
            }
            if (problems.Count > 0 && !player.Told)
            {
                player.Told = true;
                reports.Add($"[{session.Scene.NameOf(player.Root) ?? "Object"}] its Keyframe Animation cannot set everything: {string.Join("; ", problems)}");
            }
        }
        foreach (var report in reports)
        {
            session.Log(Level.Error, report);
        }
    }
}
