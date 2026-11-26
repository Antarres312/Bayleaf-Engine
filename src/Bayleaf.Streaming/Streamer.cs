using Bayleaf.Core;
using Bayleaf.Core.Geometry;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Streaming;

/// <summary>A Streamed Area in the playing scene, as this step sees it.</summary>
public sealed record Area(StableId Id, Double3 At, string Scene, double LoadDistance, double KeepMargin);

public enum StateKind
{
    /// <summary>Waiting for its turn to be read.</summary>
    Waiting,

    /// <summary>Its file is being read.</summary>
    Reading,

    /// <summary>Added: the top objects of its scene, under the area's object.</summary>
    Here,

    /// <summary>Its scene could not be read or added; tried again once the cameras leave and come back, or its scene changes.</summary>
    Failed,
}

/// <summary>Where an area that is not away is.</summary>
public sealed record AreaState(StateKind Kind, IReadOnlyList<StableId> Objects, string? Problem)
{
    public static readonly AreaState Waiting = new(StateKind.Waiting, [], null);
    public static readonly AreaState Reading = new(StateKind.Reading, [], null);

    /// <summary>A one-word name for agents: <c>waiting</c>, <c>reading</c>, <c>here</c> or <c>failed</c>.</summary>
    public string Word => Kind switch
    {
        StateKind.Waiting => "waiting",
        StateKind.Reading => "reading",
        StateKind.Here => "here",
        _ => "failed",
    };
}

/// <summary>What the engine does after <see cref="Streamer.Plan"/>.</summary>
public sealed record Plan(List<(StableId Area, string Scene)> Read, List<(StableId Area, IReadOnlyList<StableId> Objects)> Remove);

/// <summary>
/// Optional world streaming. A world is split into areas, each kept in its own scene file and marked in
/// the playing scene by a Streamed Area component. The streamer decides from the cameras' places which
/// areas should be there, reads their files on worker threads so play does not wait, and hands back the
/// scenes that arrived for the engine to add. It keeps a margin so an area at the edge does not come and
/// go every step.
/// </summary>
public sealed class Streamer
{
    /// <summary>Most scene files read at once; the others wait their turn.</summary>
    public const int MaxReading = 4;

    private sealed class Entry(string scene)
    {
        public string Scene { get; } = scene;

        public AreaState State { get; set; } = AreaState.Waiting;

        public Task<Result<SceneModel, string>>? Reading { get; set; }
    }

    private readonly SortedDictionary<StableId, Entry> entries = [];
    private readonly LinkedList<(StableId Id, string Path)> queue = new();

    /// <summary>
    /// Compares the areas with the cameras' places. An area comes when a camera is within its Load
    /// Distance and goes when every camera is beyond Load Distance plus Keep Margin; areas whose object
    /// or component went, or whose scene changed, go too. With no camera, none comes or goes by distance.
    /// </summary>
    public Plan PlanFor(IReadOnlyList<Area> areas, IReadOnlyList<Double3> cameras)
    {
        var plan = new Plan([], []);
        var current = areas.ToDictionary(a => a.Id);
        var gone = entries.Where(e => !current.TryGetValue(e.Key, out var area) || area.Scene != e.Value.Scene).Select(e => e.Key).ToList();
        foreach (var id in gone)
        {
            plan.Remove.Add((id, Forget(id)));
        }
        if (cameras.Count == 0)
        {
            return plan;
        }
        foreach (var area in areas)
        {
            if (area.Scene.Length == 0)
            {
                continue;
            }
            var nearest = cameras.Select(c => (c - area.At).Length).DefaultIfEmpty(double.PositiveInfinity).Min();
            var known = entries.ContainsKey(area.Id);
            if (!known && nearest <= area.LoadDistance)
            {
                entries[area.Id] = new Entry(area.Scene);
                plan.Read.Add((area.Id, area.Scene));
            }
            else if (known && nearest > area.LoadDistance + area.KeepMargin)
            {
                plan.Remove.Add((area.Id, Forget(area.Id)));
            }
        }
        return plan;
    }

    /// <summary>Starts reading an area's scene file (already checked to be allowed), now or when a reader is free.</summary>
    public void Read(StableId id, string path)
    {
        if (entries.ContainsKey(id))
        {
            queue.AddLast((id, path));
            StartWaiting();
        }
    }

    /// <summary>Marks an area as failed: its scene could not be read or added.</summary>
    public void Fail(StableId id, string why)
    {
        if (entries.TryGetValue(id, out var entry))
        {
            entry.State = new AreaState(StateKind.Failed, [], why);
            entry.Reading = null;
        }
        RemoveQueued(id);
        StartWaiting();
    }

    /// <summary>Records the objects an arrived area's scene became.</summary>
    public void Placed(StableId id, IReadOnlyList<StableId> roots)
    {
        if (entries.TryGetValue(id, out var entry))
        {
            entry.State = new AreaState(StateKind.Here, roots, null);
        }
    }

    /// <summary>The scenes read since the last call, without waiting. Each is still to be added, or failed.</summary>
    public List<(StableId Area, Result<SceneModel, string> Scene)> Arrived()
    {
        var output = new List<(StableId, Result<SceneModel, string>)>();
        foreach (var (id, entry) in entries)
        {
            if (entry.Reading is { IsCompleted: true } reading)
            {
                entry.Reading = null;
                output.Add((id, reading.Result));
            }
        }
        Settle(output);
        return output;
    }

    /// <summary>Waits until no area is waiting or being read, or <paramref name="timeout"/> passes, and returns what arrived.</summary>
    public List<(StableId Area, Result<SceneModel, string> Scene)> Wait(TimeSpan timeout)
    {
        var deadline = DateTime.UtcNow + timeout;
        var output = Arrived();
        while (entries.FirstOrDefault(e => e.Value.Reading is not null) is { Value: not null } found)
        {
            var reading = found.Value.Reading!;
            var left = deadline - DateTime.UtcNow;
            if (left < TimeSpan.Zero || !reading.Wait(left))
            {
                break;
            }
            found.Value.Reading = null;
            var one = new List<(StableId, Result<SceneModel, string>)> { (found.Key, reading.Result) };
            Settle(one);
            output.AddRange(one);
        }
        return output;
    }

    /// <summary>How many areas are waiting or being read.</summary>
    public int Busy => entries.Values.Count(e => e.State.Kind is StateKind.Waiting or StateKind.Reading);

    /// <summary>Every area that is not away, with its scene and state, in id order.</summary>
    public IEnumerable<(StableId Area, string Scene, AreaState State)> States => entries.Select(e => (e.Key, e.Value.Scene, e.Value.State));

    /// <summary>Forgets every area, as when the playing scene is replaced.</summary>
    public void Clear()
    {
        entries.Clear();
        queue.Clear();
    }

    private void RemoveQueued(StableId id)
    {
        for (var node = queue.First; node is not null;)
        {
            var next = node.Next;
            if (node.Value.Id == id)
            {
                queue.Remove(node);
            }
            node = next;
        }
    }

    private IReadOnlyList<StableId> Forget(StableId id)
    {
        RemoveQueued(id);
        var roots = entries.Remove(id, out var entry) && entry.State.Kind == StateKind.Here ? entry.State.Objects : [];
        StartWaiting();
        return roots;
    }

    private void Settle(List<(StableId Id, Result<SceneModel, string> Scene)> arrived)
    {
        foreach (var (id, result) in arrived)
        {
            if (entries.TryGetValue(id, out var entry) && result.IsError)
            {
                entry.State = new AreaState(StateKind.Failed, [], result.Error);
            }
        }
        StartWaiting();
    }

    private void StartWaiting()
    {
        while (entries.Values.Count(e => e.Reading is not null) < MaxReading && queue.First is { } first)
        {
            queue.RemoveFirst();
            var (id, path) = first.Value;
            if (!entries.TryGetValue(id, out var entry))
            {
                continue;
            }
            entry.State = AreaState.Reading;
            entry.Reading = Task.Run(() => SceneModel.Load(path).MapError(e => e.Message));
        }
    }
}
