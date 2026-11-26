using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Streaming;

/// <summary>
/// World streaming in play mode: Streamed Area components add their scenes under themselves, read on
/// worker threads, when a camera comes near, and remove them again once every camera has gone. An app
/// turns it on with <c>session.Use(new StreamingModule())</c>.
/// </summary>
public sealed class StreamingModule : IStreamingModule
{
    private Streamer streamer = new();
    private bool told;

    public string Name => "world streaming";

    public void Started(IPlayWorld world)
    {
        streamer = new Streamer();
        told = false;
        Update(world);
    }

    public void Stepped(IPlayWorld world, float seconds) => Update(world);

    public void Stopped() => streamer = new Streamer();

    public IReadOnlyList<StreamedState> Areas() =>
        [.. streamer.States.Select(s => new StreamedState(s.Area, s.Scene, s.State.Word, s.State.Kind == StateKind.Here ? s.State.Objects : [], s.State.Kind == StateKind.Failed ? s.State.Problem : null))];

    public (int Here, int Busy) Wait(IPlayWorld world, TimeSpan timeout)
    {
        Place(world, streamer.Wait(timeout));
        return (streamer.States.Count(s => s.State.Kind == StateKind.Here), streamer.Busy);
    }

    /// <summary>The Streamed Areas and the cameras' places in the scene.</summary>
    private static (List<Area> Areas, List<Double3> Cameras) Gather(SceneModel scene)
    {
        var (areas, cameras) = (new List<Area>(), new List<Double3>());
        foreach (var (id, matrix) in scene.WorldMatrices())
        {
            var at = matrix.WAxis.Xyz;
            if (scene.TryGet<StreamedArea>(id, out var area))
            {
                areas.Add(new Area(id, at, area.Scene, area.LoadDistance, area.KeepMargin));
            }
            if (scene.TryGet<Camera>(id, out _))
            {
                cameras.Add(at);
            }
        }
        return (areas, cameras);
    }

    /// <summary>Adds the areas that arrived, then brings and takes areas as the cameras moved.</summary>
    private void Update(IPlayWorld world)
    {
        Place(world, streamer.Arrived());
        var (areas, cameras) = Gather(world.Scene);
        if (areas.Count > 0 && cameras.Count == 0 && !told)
        {
            told = true;
            world.Log(false, "Streamed Areas wait for a camera: the scene has none.");
        }
        var plan = streamer.PlanFor(areas, cameras);
        foreach (var root in plan.Remove.SelectMany(r => r.Objects))
        {
            if (world.Scene.Contains(root))
            {
                world.Scene.DeleteObject(root);
            }
        }
        foreach (var (id, scene) in plan.Read)
        {
            var path = world.Readable(scene);
            if (path.IsOk)
            {
                streamer.Read(id, path.Value);
            }
            else
            {
                Failure(world, id, scene, path.Error);
                streamer.Fail(id, path.Error);
            }
        }
    }

    private void Place(IPlayWorld world, List<(StableId Area, Result<SceneModel, string> Scene)> arrived)
    {
        var scenes = streamer.States.ToDictionary(s => s.Area, s => s.Scene);
        foreach (var (id, read) in arrived)
        {
            var scene = scenes.GetValueOrDefault(id, "");
            Result<IReadOnlyList<StableId>, string> added = read.IsError
                ? Result.Fail(read.Error)
                : !world.Scene.Contains(id) ? Result.Fail("its object is gone") : world.AddScene(read.Value, id);
            if (added.IsOk)
            {
                streamer.Placed(id, added.Value);
            }
            else
            {
                Failure(world, id, scene, added.Error);
                streamer.Fail(id, added.Error);
            }
        }
    }

    private static void Failure(IPlayWorld world, StableId id, string scene, string why) =>
        world.Log(true, $"[{world.Scene.NameOf(id) ?? "Object"} · Streamed Area] {scene} could not be added: {why}");
}
