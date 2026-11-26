using System.Globalization;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Engine;

/// <summary>A prefab open on its own (<c>prefab.open</c>), and the scene it stands in front of.</summary>
internal sealed record PrefabEdit(string Source, SceneModel Scene, bool Dirty, List<Session.HistoryStep> Undo, List<Session.HistoryStep> Redo);

/// <summary>
/// Prefabs: an object and its children saved to a <c>.bprefab</c> file, to place again and again. A
/// placed copy keeps a <c>prefab</c> link to its file. Apply writes a copy's objects to the file and
/// updates every other copy in the scene; opening a scene updates every copy from its file; Revert
/// reads the file again and drops the copy's own changes. Each copy keeps its own name and transform,
/// and the component changes made to it (its overrides) survive updates. A prefab may hold copies of
/// other prefabs: they update from their own files, inside the copy.
/// </summary>
internal static class PrefabCommands
{
    public const string Format = "bayleaf.prefab";
    public const string Extension = "bprefab";

    /// <summary>Most prefabs inside one another.</summary>
    private const int MaxNesting = 16;

    /// <summary>Components a copy's own root keeps from the scene, not the file.</summary>
    private static readonly string[] Kept = ["name", "transform", "prefab"];

    public static IEnumerable<CommandSpec> All() =>
    [
        Spec("scene.prefab.create", "Save as Prefab", "{id, path}: saves the object and its children to a .bprefab file and links the object to it", Create),
        Spec("scene.prefab.place", "Place Prefab", "{path, parent?, index?}: adds a linked copy of a .bprefab file; returns {id}", Place),
        Spec("scene.prefab.apply", "Apply to Prefab", "{id}: writes this copy to its prefab file and updates the other copies in the scene; returns {updated}", Apply),
        Spec("scene.prefab.revert", "Revert to Prefab", "{id}: replaces this copy's objects with the prefab file's, keeping its name and transform", Revert),
        new()
        {
            Id = "prefab.open", Label = "Open Prefab",
            Params = "{path}: edits the prefab on its own: the scene and its undo history wait underneath, and prefab.close goes back; returns {source, depth}",
            Enabled = static s => s.IsPlaying ? "stop play mode first" : null, Run = Open,
        },
        new()
        {
            Id = "prefab.save", Label = "Save Prefab", Params = "none: writes the open prefab to its file; returns {path}",
            Enabled = static s => s.PrefabEdits.Count == 0 ? "no prefab is open" : null, Run = SavePrefab,
        },
        new()
        {
            Id = "prefab.close", Label = "Back from Prefab",
            Params = "{discard?}: goes back to the scene (or the prefab) underneath, whose copies update from the file; refuses unsaved changes unless discard",
            Enabled = static s => s.PrefabEdits.Count == 0 ? "no prefab is open" : null, Run = Close,
        },
    ];

    private static CommandSpec Spec(string id, string label, string parameters, Func<Session, JsonValue, JsonValue> run) =>
        new() { Id = id, Label = label, Params = parameters, Mutates = Mutates.Scene, Run = run };

    private static JsonObject Open(Session session, JsonValue p)
    {
        var path = Params.Of(p, "path").Required("path", JsonRead.Str);
        var source = LinkPath(session, Path.Combine(session.ScriptDir, path));
        if (session.PrefabEdits.Exists(e => e.Source == source))
        {
            throw Stop.Failed($"{source} is already open");
        }
        if (session.PrefabEdits.Count >= MaxNesting)
        {
            throw Stop.Failed($"prefabs open inside one another at most {MaxNesting} deep");
        }
        var objects = ReadPrefab(session, source);
        // The objects keep their ids from the file, so saving keeps them too.
        var root = objects.Count > 0 ? objects[0].Id : throw Stop.Failed("the prefab is empty");
        var scene = SceneModel.FromDoc(new SceneDoc(SceneDoc.FormatName, SceneDoc.CurrentVersion, SceneSettings.Default, objects)).OrFail();
        // Around the origin: copies keep their own transform anyway.
        if (scene.TryGet<Transform>(root, out var transform))
        {
            scene.Set(root, transform with { Position = Core.Geometry.Double3.Zero }).OrFail();
        }
        session.PrefabEdits.Add(new PrefabEdit(source, session.Scene, session.Dirty, session.Undo, session.Redo));
        session.Scene = scene;
        session.Dirty = false;
        session.Undo = [];
        session.Redo = [];
        // Copies of other prefabs inside it come from their own files.
        var problems = new List<string>();
        foreach (var nested in NestedCopies(session.Scene, root))
        {
            try
            {
                Refresh(session, nested, true, [source]);
            }
            catch (CommandException e)
            {
                problems.Add(e.Error.Message);
            }
        }
        return J.Obj(("source", J.S(source)), ("depth", J.N(session.PrefabEdits.Count)), ("problems", J.Strings(problems)));
    }

    private static JsonObject SavePrefab(Session session, JsonValue p)
    {
        Params.None(p);
        var source = session.PrefabEdits.Count > 0 ? session.PrefabEdits[^1].Source : throw Stop.Failed("no prefab is open");
        var roots = session.Scene.Children(null);
        if (roots.Count != 1)
        {
            throw Stop.Failed($"a prefab has one root object, and this one has {roots.Count}: put the others under one");
        }
        var path = WritePrefab(session, Path.Combine(session.ScriptDir, source), session.Scene, roots[0]);
        session.Dirty = false;
        return J.Obj(("path", J.S(path)));
    }

    private static JsonObject Close(Session session, JsonValue p)
    {
        var discard = Params.Of(p, "discard").Optional("discard", JsonRead.Bool, false);
        if (session.Dirty && !discard)
        {
            throw Stop.Failed("the prefab has unsaved changes: save it (prefab.save), or go back with discard");
        }
        var edit = session.PrefabEdits.Count > 0 ? session.PrefabEdits[^1] : throw Stop.Failed("no prefab is open");
        session.PrefabEdits.RemoveAt(session.PrefabEdits.Count - 1);
        session.Scene = edit.Scene;
        session.Dirty = edit.Dirty;
        session.Undo = edit.Undo;
        session.Redo = edit.Redo;
        // The copies underneath follow the file, keeping their own changes.
        var problems = RefreshAll(session);
        foreach (var problem in problems)
        {
            session.Log(Level.Error, problem);
        }
        return J.Obj(("depth", J.N(session.PrefabEdits.Count)), ("problems", J.Strings(problems)));
    }

    /// <summary>The path as stored in a link: relative to the project folder when inside it, with <c>/</c>.</summary>
    internal static string LinkPath(Session session, string path)
    {
        var baseDir = Canonical(session.ScriptDir) ?? session.ScriptDir;
        var full = Canonical(path) ?? path;
        var relative = Path.GetRelativePath(baseDir, full);
        if (!Path.IsPathRooted(relative) && !relative.StartsWith("..", StringComparison.Ordinal) && Canonical(path) is not null)
        {
            return relative.Replace('\\', '/');
        }
        return path.Replace('\\', '/');
    }

    /// <summary>The full path of an existing file or folder, as the Rust engine's canonical form; null when it does not exist.</summary>
    private static string? Canonical(string path)
    {
        try
        {
            return File.Exists(path) || Directory.Exists(path) ? Path.GetFullPath(path).TrimEnd(Path.DirectorySeparatorChar) : null;
        }
        catch (Exception e) when (e is IOException or ArgumentException or NotSupportedException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    internal static List<ObjectDoc> ReadPrefab(Session session, string source)
    {
        var path = session.Allowed(Path.Combine(session.ScriptDir, source), Access.Read);
        var text = Files.ReadText(path);
        if (text.IsError)
        {
            throw Stop.Failed($"cannot read {path}: {text.Error}");
        }
        var parsed = JsonReader.Parse(text.Value);
        var file = parsed.IsOk
            ? JsonRead.Try(() =>
            {
                var f = JsonFields.Of(parsed.Value, "struct PrefabFile", true, "format", "version", "objects");
                return (Format: f.Required("format", JsonRead.Str), Version: f.Required("version", JsonRead.U32), Objects: f.Required("objects", v => JsonRead.List(v, ObjectDoc.FromJson)));
            })
            : Result.Fail(parsed.Error);
        if (file.IsError)
        {
            throw Stop.Failed($"{path} is not a prefab: {file.Error}");
        }
        if (file.Value.Format != Format || file.Value.Version is < 1 or > SceneDoc.CurrentVersion)
        {
            throw Stop.Failed($"{path} is not a prefab this engine can read");
        }
        return file.Value.Objects.Count > 0 && file.Value.Objects[0].Parent is null ? file.Value.Objects : throw Stop.Failed($"{path} has no root object");
    }

    private static string WritePrefab(Session session, string path, SceneModel scene, StableId id)
    {
        path = session.Allowed(path, Access.Write);
        var objects = scene.Export([id]).OrFail().Select(o => o.Copy()).ToList();
        // The file holds the object itself, not its link to the file.
        if (objects.Count > 0)
        {
            objects[0].Components.Remove("prefab");
        }
        var text = new JsonObject
        {
            { "format", new JsonString(Format) },
            { "version", JsonNumber.Of((ulong)SceneDoc.CurrentVersion) },
            { "objects", new JsonArray(objects.Select(o => (JsonValue)o.ToJson())) },
        }.ToPretty();
        if (Path.GetDirectoryName(path) is { Length: > 0 } parent)
        {
            Files.CreateDirectory(parent).MapError(e => $"cannot make {parent}: {e}").OrFail();
        }
        Files.Write(path, System.Text.Encoding.UTF8.GetBytes(text)).MapError(e => $"cannot write {path}: {e}").OrFail();
        return path;
    }

    private static JsonValue Create(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "path");
        var id = f.Required("id", JsonRead.Id);
        var given = f.Required("path", JsonRead.Str);
        if (!string.Equals(Path.GetExtension(given).TrimStart('.'), Extension, StringComparison.Ordinal))
        {
            throw Stop.BadParams($"prefab files end in .{Extension}");
        }
        var path = Path.Combine(session.ScriptDir, given);
        RefuseSelfCopy(session, id, LinkPath(session, path));
        var written = WritePrefab(session, path, session.Scene, id);
        var source = LinkPath(session, written);
        session.Scene.Set(id, new Prefab(source)).OrFail();
        return J.Obj(("path", J.S(written)), ("source", J.S(source)));
    }

    private static JsonValue Place(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "parent", "index");
        var given = f.Required("path", JsonRead.Str);
        var parent = Params.OptionalId(f, "parent");
        int? index = f.NullableValue("index", Params.Index);
        var source = LinkPath(session, Path.Combine(session.ScriptDir, given));
        var objects = ReadPrefab(session, source);
        var tops = session.Scene.Import(objects, parent, index).OrFail();
        var id = tops.Count > 0 ? tops[0] : throw Stop.Failed("the prefab is empty");
        session.Scene.Set(id, new Prefab(source)).OrFail();
        foreach (var nested in NestedCopies(session.Scene, id))
        {
            Refresh(session, nested, true, [source]);
        }
        return J.Obj(("id", J.Id(id)));
    }

    private static string SourceOf(Session session, StableId id) =>
        session.Scene.TryGet<Prefab>(id, out var link) ? link.Source : throw Stop.Failed("this object is not a placed prefab");

    private static JsonValue Apply(Session session, JsonValue p)
    {
        var id = Params.Of(p, "id").Required("id", JsonRead.Id);
        var source = SourceOf(session, id);
        RefuseSelfCopy(session, id, source);
        WritePrefab(session, Path.Combine(session.ScriptDir, source), session.Scene, id);
        // This copy now is the prefab: it has no changes of its own.
        session.Scene.Set(id, new Prefab(source)).OrFail();
        var others = session.Scene.WorldMatrices().Select(m => m.Id)
            .Where(o => o != id && session.Scene.TryGet<Prefab>(o, out var link) && link.Source == source).ToList();
        foreach (var other in others)
        {
            // An earlier update may have rebuilt the copy this one sat in.
            if (session.Scene.Contains(other))
            {
                Refresh(session, other, true, []);
            }
        }
        return J.Obj(("updated", J.N(others.Count)));
    }

    private static JsonValue Revert(Session session, JsonValue p)
    {
        var id = Params.Of(p, "id").Required("id", JsonRead.Id);
        var source = SourceOf(session, id);
        session.Scene.Set(id, new Prefab(source)).OrFail();
        Refresh(session, id, false, []);
        return new JsonObject();
    }

    /// <summary>A prefab may not hold a copy of itself.</summary>
    private static void RefuseSelfCopy(Session session, StableId id, string source)
    {
        if (Order(session.Scene, id).Skip(1).Any(o => session.Scene.TryGet<Prefab>(o, out var link) && link.Source == source))
        {
            throw Stop.Failed($"{source} would hold a copy of itself");
        }
    }

    /// <summary><paramref name="root"/> and everything below it, in hierarchy order.</summary>
    private static List<StableId> Order(SceneModel scene, StableId root) => scene.Export([root]) is { IsOk: true } e ? e.Value.Select(o => o.Id).ToList() : [];

    /// <summary>The nearest copy at or above <paramref name="id"/>, and its index among that copy's objects.</summary>
    private static (StableId Copy, int Index)? CopyOf(SceneModel scene, StableId id)
    {
        StableId? at = id;
        while (at is { } obj)
        {
            if (scene.TryGet<Prefab>(obj, out _))
            {
                var index = Order(scene, obj).IndexOf(id);
                return index >= 0 ? (obj, index) : null;
            }
            at = scene.Parent(obj);
        }
        return null;
    }

    /// <summary>Copies inside <paramref name="root"/> that are not inside another copy inside it.</summary>
    private static List<StableId> NestedCopies(SceneModel scene, StableId root) =>
        Order(scene, root).Skip(1).Where(id => scene.TryGet<Prefab>(id, out _) && CopyOf(scene, scene.Parent(id) ?? root) is { } owner && owner.Copy == root).ToList();

    /// <summary>
    /// Records that a component of an object was changed in the scene, so the copy it belongs to keeps
    /// the change when it is updated from its file.
    /// </summary>
    internal static void NoteOverride(SceneModel scene, StableId id, string component)
    {
        if (OverrideOwner(scene, id, component) is var (root, index))
        {
            AddOverride(scene, root, index, component);
        }
    }

    /// <summary>The copy that keeps a component of <paramref name="id"/> as its own change, and the object's index among its objects.</summary>
    private static (StableId Root, int Index)? OverrideOwner(SceneModel scene, StableId id, string component)
    {
        if (component == "prefab" || CopyOf(scene, id) is not var (root, index))
        {
            return null;
        }
        if (index == 0 && Kept.Contains(component))
        {
            if (scene.Parent(root) is not { } parent || CopyOf(scene, parent) is not var (outer, _))
            {
                return null;
            }
            var at = Order(scene, outer).IndexOf(root);
            return at >= 0 ? (outer, at) : null;
        }
        return (root, index);
    }

    /// <summary>What the prefab file gives a component of an object: the file and the value, or null.</summary>
    private static (string Source, JsonValue Value)? PrefabValue(Session session, StableId id, string component)
    {
        if (OverrideOwner(session.Scene, id, component) is not var (owner, index) || !session.Scene.TryGet<Prefab>(owner, out var link))
        {
            return null;
        }
        try
        {
            var objects = ReadPrefab(session, link.Source);
            return index < objects.Count && objects[index].Components.TryGetValue(component, out var value) ? (link.Source, value) : null;
        }
        catch (CommandException)
        {
            return null;
        }
    }

    /// <summary>The prefab's value for a Reset: the whole component, or with <paramref name="script"/> that script's entry when the file has the same script there.</summary>
    internal static (string Source, JsonValue Value)? ResetValues(Session session, StableId id, string component, int? script)
    {
        if (PrefabValue(session, id, component) is not var (source, value))
        {
            return null;
        }
        if (script is not { } index)
        {
            return (source, value);
        }
        var file = JsonRead.Try(() => Scripts.Read(value));
        if (file.IsError || !session.Scene.TryGet<Scripts>(id, out var current) || index >= current.Items.Count || index >= file.Value.Items.Count)
        {
            return null;
        }
        var theirs = file.Value.Items[index];
        return theirs.Module == current.Items[index].Module ? (source, theirs.Write().Canonical()) : null;
    }

    /// <summary>Drops the override of a component once it matches the prefab file again, so changes to the file reach it.</summary>
    internal static void ForgetOverride(Session session, StableId id, string component)
    {
        if (OverrideOwner(session.Scene, id, component) is not var (root, index))
        {
            return;
        }
        var theirs = PrefabValue(session, id, component) is var (_, value) ? Stored(component, value) : null;
        var same = theirs is not null && session.Scene.Component(id, component) is { IsOk: true } now && now.Value.Equals(theirs);
        if (!session.Scene.TryGet<Prefab>(root, out var link))
        {
            return;
        }
        var key = Key(index, component);
        if (same && link.Overrides.Contains(key))
        {
            session.Scene.Set(root, link with { Overrides = link.Overrides.Where(k => k != key).ToList() });
        }
    }

    private static string Key(int index, string component) => string.Create(CultureInfo.InvariantCulture, $"{index}/{component}");

    /// <summary>A component's value as the scene stores it, so fields a file leaves out compare as their defaults.</summary>
    private static JsonValue? Stored(string component, JsonValue value)
    {
        var probe = new SceneModel();
        if (!probe.CreateObject(null, "probe").TryGet(out var obj, out _))
        {
            return null;
        }
        if (probe.Component(obj, component).IsError && probe.AddComponent(obj, component).IsError)
        {
            return null;
        }
        return probe.SetComponent(obj, component, value).IsOk && probe.Component(obj, component) is { IsOk: true } stored ? stored.Value : null;
    }

    private static void AddOverride(SceneModel scene, StableId root, int index, string component)
    {
        if (!scene.TryGet<Prefab>(root, out var link))
        {
            return;
        }
        var key = Key(index, component);
        if (!link.Overrides.Contains(key) && link.Overrides.Count < Prefab.MaxOverrides)
        {
            scene.Set(root, link with { Overrides = [.. link.Overrides, key] });
        }
    }

    /// <summary>One copy's overrides with their values (null: the component was removed).</summary>
    private sealed record KeptCopy(int At, string Source, List<(int Index, string Component, JsonValue? Value)> Values);

    /// <summary>
    /// Updates copy <paramref name="root"/> from its file. With <paramref name="keep"/>, the overrides of
    /// it and of the copies inside it are put back afterwards; the copies inside it update from their
    /// own files first. <paramref name="visited"/> holds the files being updated around it.
    /// </summary>
    private static void Refresh(Session session, StableId root, bool keep, List<string> visited)
    {
        var source = SourceOf(session, root);
        if (visited.Contains(source) || visited.Count > MaxNesting)
        {
            throw Stop.Failed($"{source} holds a copy of itself, or prefabs nest more than {MaxNesting} deep");
        }
        var objects = ReadPrefab(session, source);
        var kept = keep ? Capture(session.Scene, root) : [];
        ReplaceContents(session.Scene, root, objects);
        visited.Add(source);
        foreach (var nested in NestedCopies(session.Scene, root))
        {
            Refresh(session, nested, true, visited);
        }
        visited.RemoveAt(visited.Count - 1);
        var scene = session.Scene;
        var now = Order(scene, root);
        foreach (var copy in kept)
        {
            if (copy.At >= now.Count)
            {
                continue;
            }
            var target = now[copy.At];
            if (!scene.TryGet<Prefab>(target, out var link) || link.Source != copy.Source)
            {
                continue;
            }
            var overrides = link.Overrides.ToList();
            var targetObjects = Order(scene, target);
            foreach (var (index, component, value) in copy.Values)
            {
                if (index >= targetObjects.Count)
                {
                    continue;
                }
                var obj = targetObjects[index];
                // Best effort: a change that no longer fits the file is dropped.
                bool applied;
                if (value is not null)
                {
                    if (scene.Component(obj, component).IsError)
                    {
                        scene.AddComponent(obj, component);
                    }
                    applied = scene.SetComponent(obj, component, value).IsOk;
                }
                else
                {
                    applied = scene.Component(obj, component).IsError || scene.RemoveComponent(obj, component).IsOk;
                }
                var key = Key(index, component);
                if (applied && !overrides.Contains(key))
                {
                    overrides.Add(key);
                }
            }
            scene.Set(target, link with { Overrides = overrides });
        }
    }

    /// <summary>The overrides of copy <paramref name="root"/> and of the copies inside it, with their current values.</summary>
    private static List<KeptCopy> Capture(SceneModel scene, StableId root)
    {
        var all = Order(scene, root);
        var kept = new List<KeptCopy>();
        for (var at = 0; at < all.Count; at++)
        {
            if (!scene.TryGet<Prefab>(all[at], out var link))
            {
                continue;
            }
            var objects = Order(scene, all[at]);
            var values = link.ParsedOverrides().Where(o => o.Index < objects.Count)
                .Select(o => (o.Index, o.Component, scene.Component(objects[o.Index], o.Component) is { IsOk: true } c ? c.Value : null)).ToList();
            kept.Add(new KeptCopy(at, link.Source, values));
        }
        return kept;
    }

    /// <summary>Updates every copy in the scene from its file, keeping overrides; returns the problems with files that cannot be read.</summary>
    internal static List<string> RefreshAll(Session session)
    {
        var scene = session.Scene;
        var outermost = scene.WorldMatrices().Select(m => m.Id)
            .Where(id => scene.TryGet<Prefab>(id, out _) && (scene.Parent(id) is not { } parent || CopyOf(scene, parent) is null)).ToList();
        var problems = new List<string>();
        foreach (var id in outermost)
        {
            try
            {
                Refresh(session, id, true, []);
            }
            catch (CommandException e)
            {
                problems.Add(e.Error.Message);
            }
        }
        return problems;
    }

    /// <summary>Makes <paramref name="id"/> and its children match the prefab objects, keeping its own name, transform, link and place.</summary>
    private static void ReplaceContents(SceneModel scene, StableId id, List<ObjectDoc> objects)
    {
        if (objects.Count == 0)
        {
            return;
        }
        var root = objects[0];
        foreach (var child in scene.Children(id).ToList())
        {
            scene.DeleteObject(child).OrFail();
        }
        foreach (var (name, _) in scene.Components(id).OrFail().ToList())
        {
            if (!Kept.Contains(name) && !root.Components.ContainsKey(name))
            {
                scene.RemoveComponent(id, name).OrFail();
            }
        }
        foreach (var (name, value) in root.Components.Where(c => !Kept.Contains(c.Key)))
        {
            if (scene.Component(id, name).IsError)
            {
                scene.AddComponent(id, name).OrFail();
            }
            scene.SetComponent(id, name, value).OrFail();
        }
        // The root's children become top-level objects of the import, under the copy.
        var children = objects.Skip(1).Select(o => o.Parent == root.Id ? o with { Parent = null } : o).ToList();
        scene.Import(children, id, null).OrFail();
    }
}
