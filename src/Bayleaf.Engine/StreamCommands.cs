using System.Globalization;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using EnvironmentComponent = Bayleaf.Scene.Components.Environment;
using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Engine;

/// <summary>
/// <c>stream.split</c>: cuts the open scene into Streamed Areas on a grid. Streaming them in play mode
/// (<c>stream.areas</c>, <c>stream.wait</c>) comes with play mode.
/// </summary>
internal static class StreamCommands
{
    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "stream.split", Label = "Split into Streamed Areas…", Menu = "Object/Split into Streamed Areas…",
            Params = "{cellSize, folder, loadDistance?, keepMargin?}: moves the open scene's top objects into one .bscene per grid cell (cellSize wide, by X and Z) in folder (relative to the project; the files must not be there yet), and puts a Streamed Area for each cell under a new Streamed World object. Cameras, sun lights, environment, game rules, player starts and players stay. loadDistance defaults to 1.5 cells, keepMargin to a quarter cell; returns {world, areas: [{id, scene, objects}], moved}",
            Prompt = Prompt.Form(new PromptField("cellSize", "Cell Size", "100", true), new PromptField("folder", "Folder", "cells", false)),
            Mutates = Mutates.Scene, Enabled = Session.NotPlaying, Run = Split,
        },
    ];

    /// <summary>Whether a top object belongs everywhere rather than in one cell.</summary>
    private static bool Stays(SceneModel scene, StableId id) =>
        scene.TryGet<Camera>(id, out _) || scene.TryGet<DirectionalLight>(id, out _) || scene.TryGet<EnvironmentComponent>(id, out _) || scene.TryGet<GameRules>(id, out _)
        || scene.TryGet<PlayerStart>(id, out _) || scene.TryGet<PlayerControl>(id, out _) || scene.TryGet<StreamedArea>(id, out _);

    private static JsonObject Split(Session session, JsonValue p)
    {
        var f = Params.Of(p, "cellSize", "folder", "loadDistance", "keepMargin");
        var size = f.Required("cellSize", JsonRead.F64);
        var given = f.Required("folder", JsonRead.Str);
        double? load = f.NullableValue("loadDistance", JsonRead.F64);
        double? margin = f.NullableValue("keepMargin", JsonRead.F64);
        if (!(double.IsFinite(size) && size >= 1 && size <= 1.0e6))
        {
            throw Stop.BadParams("cellSize must be between 1 and 1000000");
        }
        var area = new StreamedArea("", load ?? size * 1.5, margin ?? size * 0.25);
        var folder = given.Trim('/', '\\').Replace('\\', '/');
        if (folder.Length == 0 || folder.Split('/').Any(part => part is "" or "." or ".."))
        {
            throw Stop.BadParams("folder must be a folder inside the project");
        }
        var scene = session.Scene;
        var cells = new SortedDictionary<(long X, long Z), List<StableId>>();
        foreach (var id in scene.Children(null))
        {
            if (Stays(scene, id))
            {
                continue;
            }
            var at = scene.TryGet<Transform>(id, out var t) ? t.Position : Double3.Zero;
            var key = ((long)Math.Floor(at.X / size), (long)Math.Floor(at.Z / size));
            if (!cells.TryGetValue(key, out var list))
            {
                list = [];
                cells[key] = list;
            }
            list.Add(id);
        }
        if (cells.Count == 0)
        {
            throw Stop.Failed("nothing to split: the scene's top objects all belong everywhere (cameras, sun lights, environment, rules, players)");
        }
        // Check every file first, so a refusal leaves no files behind.
        var planned = new List<(string Relative, string Path, Double3 Centre, List<StableId> Objects)>();
        foreach (var ((x, z), objects) in cells)
        {
            var relative = string.Create(CultureInfo.InvariantCulture, $"{folder}/cell_{x}_{z}.bscene");
            var path = session.Allowed(System.IO.Path.Combine(session.ScriptDir, relative), Access.Write);
            if (File.Exists(path) || Directory.Exists(path))
            {
                throw Stop.Failed($"{relative} is already there; choose another folder");
            }
            ((ISceneComponent<StreamedArea>)(area with { Scene = relative })).Validate().OrBadParams();
            planned.Add((relative, path, new Double3((x + 0.5) * size, 0, (z + 0.5) * size), objects));
        }
        foreach (var (relative, path, centre, objects) in planned)
        {
            var cell = new SceneModel();
            var roots = cell.Import(scene.Export(objects).OrFail(), null, null).OrFail();
            foreach (var root in roots)
            {
                var transform = cell.TryGet<Transform>(root, out var t) ? t : Transform.Initial;
                cell.Set(root, transform with { Position = transform.Position - centre }).OrFail();
            }
            if (System.IO.Path.GetDirectoryName(path) is { Length: > 0 } parent)
            {
                Files.CreateDirectory(parent).MapError(e => $"cannot make {parent}: {e}").OrFail();
            }
            var saved = cell.Save(path);
            if (saved.IsError)
            {
                throw Stop.Failed($"{relative}: {saved.Error.Message}");
            }
        }
        var world = scene.CreateObject(null, "Streamed World").OrFail();
        var listed = new JsonArray();
        var moved = 0;
        foreach (var (relative, _, centre, objects) in planned)
        {
            foreach (var id in objects)
            {
                moved += scene.DeleteObject(id).OrFail();
            }
            var name = string.Create(CultureInfo.InvariantCulture, $"Cell {Text.Number(Math.Round(centre.X / size - 0.5, MidpointRounding.AwayFromZero))}, {Text.Number(Math.Round(centre.Z / size - 0.5, MidpointRounding.AwayFromZero))}");
            var id2 = scene.CreateObject(world, name).OrFail();
            scene.Set(id2, Transform.Initial with { Position = centre }).OrFail();
            scene.Set(id2, area with { Scene = relative }).OrFail();
            listed.Add(J.Obj(("id", J.Id(id2)), ("scene", J.S(relative)), ("objects", J.N(objects.Count))));
        }
        session.Log(Level.Info, string.Create(CultureInfo.InvariantCulture, $"Split {moved} objects into {listed.Count} Streamed Areas in {folder}."));
        return J.Obj(("world", J.Id(world)), ("areas", listed), ("moved", J.N(moved)));
    }
}
