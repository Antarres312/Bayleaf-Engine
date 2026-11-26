using System.Globalization;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Engine;

/// <summary>Scene commands: files, objects, hierarchy and components.</summary>
internal static class SceneCommands
{
    /// <summary><c>format</c> of copied objects (clipboard data).</summary>
    public const string ClipboardFormat = "bayleaf.objects";

    public static IEnumerable<CommandSpec> All() =>
    [
        new() { Id = "scene.new", Label = "New Scene", Menu = "File/New Scene", Shortcut = "Ctrl+N", Params = "none", Enabled = Session.NotPlaying, Run = New },
        new()
        {
            Id = "scene.open", Label = "Open Scene…", Menu = "File/Open Scene…", Shortcut = "Ctrl+O", Params = "{path}", Prompt = Prompt.OpenScene,
            Enabled = Session.NotPlaying, Run = Open,
        },
        new()
        {
            Id = "scene.save", Label = "Save Scene", Menu = "File/Save Scene", Shortcut = "Ctrl+S", Params = "none",
            Enabled = static s => Session.NotPlaying(s) ?? (s.ScenePath is not null ? null : "the scene has no file yet; use Save Scene As"), Run = Save,
        },
        new()
        {
            Id = "scene.saveAs", Label = "Save Scene As…", Menu = "File/Save Scene As…", Shortcut = "Ctrl+Shift+S", Params = "{path}", Prompt = Prompt.SaveScene,
            Enabled = Session.NotPlaying, Run = SaveAs,
        },
        new()
        {
            Id = "scene.render", Label = "Render Image…",
            Params = "{path?, width?, height?, camera?, grid?}; writes a PNG, or without a path returns it as {png: base64} (at most 2048 on a side). camera: an object with a camera component; default: the first camera in the scene, else a view framing every object",
            Run = RenderCommands.Render,
        },
        new() { Id = "scene.objects", Label = "List Objects", Params = "none; returns the object tree", Run = Objects },
        new()
        {
            Id = "scene.document", Label = "Show Scene Document",
            Params = "none; returns {path, dirty, scene}: every object and component, as the scene file holds them", Run = Document,
        },
        new() { Id = "scene.components", Label = "List Component Types", Params = "none", Run = ComponentTypes },
        new()
        {
            Id = "scene.object.create", Label = "Create Object", Menu = "Object/Create Empty Object", Shortcut = "Ctrl+Shift+N",
            Params = "{parent?, name?, index?, components?: {name: value or null}}; returns {id}", Mutates = Mutates.Scene, Run = Create,
        },
        new()
        {
            Id = "scene.object.delete", Label = "Delete Object", Menu = "Object/Delete Object", Shortcut = "Delete", Params = "{id}",
            Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene, Run = Delete,
        },
        new()
        {
            Id = "scene.object.duplicate", Label = "Duplicate", Menu = "Edit/Duplicate", Shortcut = "Ctrl+D",
            Params = "{id}; the copy, with its children, goes right after the original. Returns {id}", Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene,
            Run = Duplicate,
        },
        new()
        {
            Id = "scene.object.copy", Label = "Copy", Params = "{ids: [id, ...]}; returns the objects and their children as clipboard data for scene.object.paste",
            Run = Copy,
        },
        new()
        {
            Id = "scene.object.paste", Label = "Paste", Params = "{objects: <data from scene.object.copy>, parent?: id, index?: n}; returns the new {ids}",
            Mutates = Mutates.Scene, Run = Paste,
        },
        Move("scene.object.moveUp", "Move Up", "Object/Move Up", "Alt+ArrowUp", "{id}; moves before the previous sibling", StepKind.Up),
        Move("scene.object.moveDown", "Move Down", "Object/Move Down", "Alt+ArrowDown", "{id}; moves after the next sibling", StepKind.Down),
        Move("scene.object.indent", "Make Child of Previous", "Object/Make Child of Previous", "Alt+ArrowRight", "{id}; becomes the last child of the previous sibling", StepKind.Indent),
        Move("scene.object.outdent", "Move Out of Parent", "Object/Move Out of Parent", "Alt+ArrowLeft", "{id}; moves right after its parent", StepKind.Outdent),
        new()
        {
            Id = "scene.script.add", Label = "Add Script",
            Params = "{id, module}; module: .wasm or .bscript path relative to the project folder (or the scene file's folder outside a project); returns {index}",
            Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene, Run = ScriptAdd,
        },
        new() { Id = "scene.script.remove", Label = "Remove Script", Params = "{id, index}", Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene, Run = ScriptRemove },
        new()
        {
            Id = "scene.script.setProperty", Label = "Set Script Property",
            Params = "{id, index, name, value}: value is a number, true/false, three numbers or text, as the script declares the property (an object property: the object's name); null returns to the script's default",
            Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene, Run = ScriptSetProperty,
        },
        new()
        {
            Id = "scene.object.inspect", Label = "Inspect Object", Params = "{id}; returns {id, parent, children, components}", Prompt = Prompt.SelectedObject,
            Run = Inspect,
        },
        new()
        {
            Id = "scene.object.reparent", Label = "Move Object", Params = "{id, parent?, index?, keepWorld? = true}; parent null moves to the root",
            Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene, Run = Reparent,
        },
        new()
        {
            Id = "scene.component.set", Label = "Set Component Value", Params = "{id, component, value}", Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene,
            Run = ComponentSet,
        },
        new()
        {
            Id = "scene.component.add", Label = "Add Component", Params = "{id, component}", Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene,
            Run = ComponentAdd,
        },
        new()
        {
            Id = "scene.component.remove", Label = "Remove Component", Params = "{id, component}", Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene,
            Run = ComponentRemove,
        },
        new()
        {
            Id = "scene.component.reset", Label = "Reset Component",
            Params = "{id, component, script?}: puts every field back to its default; with script (an index, component \"scripts\") only that script's properties, to their declared defaults. On a prefab copy the prefab file's values are used instead, where the file has them. Returns {from: \"defaults\"} or {from: \"prefab\", source}",
            Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene, Run = ComponentReset,
        },
    ];

    private static JsonObject New(Session session, JsonValue p)
    {
        session.NoPrefabOpen("a new scene");
        _ = Params.Of(p);
        session.ReplaceScene(new SceneModel(), null);
        return new JsonObject();
    }

    private static string PathParam(JsonValue p) => Params.Of(p, "path").Required("path", JsonRead.Str);

    private static JsonObject Open(Session session, JsonValue p)
    {
        session.NoPrefabOpen("opening a scene");
        var path = session.Allowed(PathParam(p), Access.Read);
        var scene = SceneModel.Load(path).OrFail();
        var objects = scene.ObjectCount;
        var warnings = session.ReplaceScene(scene, path);
        return J.Obj(("objects", J.N(objects)), ("warnings", J.Strings(warnings)));
    }

    private static JsonObject Save(Session session, JsonValue p)
    {
        session.NoPrefabOpen("saving the scene");
        var path = session.ScenePath ?? throw Stop.Failed("the scene has no file yet; use Save Scene As");
        path = session.Allowed(path, Access.Write);
        session.Scene.Save(path).OrFail();
        session.Dirty = false;
        return J.Obj(("path", J.S(path)));
    }

    private static JsonObject SaveAs(Session session, JsonValue p)
    {
        session.NoPrefabOpen("saving the scene");
        var path = session.Allowed(PathParam(p), Access.Write);
        session.Scene.Save(path).OrFail();
        session.SetScenePath(path);
        session.Dirty = false;
        return J.Obj(("path", J.S(path)));
    }

    private static JsonObject Document(Session session, JsonValue p)
    {
        Params.None(p);
        return J.Obj(("path", J.S(session.ScenePath)), ("dirty", J.B(session.Dirty)), ("scene", session.Scene.ToDoc().ToJson()));
    }

    private static JsonArray Tree(SceneModel scene, StableId? parent) =>
        new(scene.Children(parent).Select(id => (JsonValue)J.Obj(("id", J.Id(id)), ("name", J.S(scene.NameOf(id))), ("children", Tree(scene, id)))));

    private static JsonArray Objects(Session session, JsonValue p) => Tree(session.Scene, null);

    private static JsonArray ComponentTypes(Session session, JsonValue p) =>
        new JsonArray(session.Scene.Registry.Kinds.Select(k => (JsonValue)J.Obj(("name", J.S(k.Name)), ("label", J.S(k.Label)), ("removable", J.B(k.Removable)))));

    private static JsonObject Create(Session session, JsonValue p)
    {
        var f = Params.Of(p, "parent", "name", "index", "components");
        var parent = Params.OptionalId(f, "parent");
        var name = f.Nullable("name", JsonRead.Str);
        int? index = f.NullableValue("index", Params.Index);
        var components = f.Optional("components", v => JsonRead.Map(v, c => c), new SortedDictionary<string, JsonValue>(Utf8Order.Instance));
        var scene = session.Scene;
        var id = scene.CreateObject(parent, name ?? "Object").OrFail();
        if (index is not null)
        {
            scene.Reparent(id, parent, index).OrFail();
        }
        foreach (var (component, value) in components)
        {
            if (scene.Component(id, component).IsError)
            {
                scene.AddComponent(id, component).OrFail();
            }
            if (value is not JsonNull)
            {
                scene.SetComponent(id, component, value.Canonical()).OrFail();
            }
        }
        return J.Obj(("id", J.Id(id)));
    }

    private static StableId IdParam(JsonValue p) => Params.Of(p, "id").Required("id", JsonRead.Id);

    private static JsonObject Delete(Session session, JsonValue p) => J.Obj(("deleted", J.N(session.Scene.DeleteObject(IdParam(p)).OrFail())));

    private static JsonObject Duplicate(Session session, JsonValue p)
    {
        var id = IdParam(p);
        var scene = session.Scene;
        var objects = scene.Export([id]).OrFail();
        var parent = scene.Parent(id);
        var at = scene.Children(parent).ToList().IndexOf(id);
        int? index = at >= 0 ? at + 1 : null;
        var tops = scene.Import(objects, parent, index).OrFail();
        var copy = tops.Count > 0 ? tops[0] : throw Stop.Failed("nothing was copied");
        var name = NextName(scene, parent, scene.NameOf(id) ?? "Object");
        scene.SetComponent(copy, "name", new JsonString(name)).OrFail();
        return J.Obj(("id", J.Id(copy)));
    }

    /// <summary><paramref name="name"/> with the lowest number after it that no sibling uses: "Crate" becomes "Crate 2".</summary>
    private static string NextName(SceneModel scene, StableId? parent, string name)
    {
        var space = name.LastIndexOf(' ');
        var baseName = space > 0 && uint.TryParse(name[(space + 1)..], NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out _) && !name[(space + 1)..].StartsWith('-')
            ? name[..space]
            : name;
        var taken = scene.Children(parent).Select(scene.NameOf).ToHashSet(StringComparer.Ordinal);
        for (var n = 2L; n < uint.MaxValue; n++)
        {
            var candidate = $"{baseName} {n}";
            if (!taken.Contains(candidate))
            {
                return candidate;
            }
        }
        return baseName;
    }

    private static JsonObject Copy(Session session, JsonValue p)
    {
        var ids = Params.Of(p, "ids").Required("ids", v => JsonRead.List(v, JsonRead.Id));
        var objects = session.Scene.Export(ids).OrFail();
        return J.Obj(("format", J.S(ClipboardFormat)), ("version", J.N(SceneDoc.CurrentVersion)), ("objects", new JsonArray(objects.Select(o => (JsonValue)o.ToJson()))));
    }

    private static JsonObject Paste(Session session, JsonValue p)
    {
        var f = Params.Of(p, "objects", "parent", "index");
        var data = f.Required("objects", v => v);
        var parent = Params.OptionalId(f, "parent");
        int? index = f.NullableValue("index", Params.Index);
        // Either the whole clipboard data or just its object list.
        var list = data switch
        {
            JsonObject o when o.Find("format") is JsonString { Value: ClipboardFormat } => o.Find("objects") ?? JsonValue.Null,
            JsonObject => throw Stop.BadParams($"objects must be a list or data with format \"{ClipboardFormat}\""),
            _ => data,
        };
        var objects = JsonRead.Try(() => JsonRead.List(list, ObjectDoc.FromJson));
        if (objects.IsError)
        {
            throw Stop.BadParams($"objects: {objects.Error}");
        }
        var ids = session.Scene.Import(objects.Value, parent, index).OrFail();
        return J.Obj(("ids", J.Ids(ids)));
    }

    private static JsonObject Inspect(Session session, JsonValue p)
    {
        var id = IdParam(p);
        var scene = session.Scene;
        var components = new JsonObject(scene.Components(id).OrFail().Select(c => new KeyValuePair<string, JsonValue>(c.Name, c.Value)));
        return J.Obj(("id", J.Id(id)), ("parent", J.Id(scene.Parent(id))), ("children", J.Ids(scene.Children(id))), ("components", components));
    }

    private enum StepKind
    {
        Up,
        Down,
        Indent,
        Outdent,
    }

    private static CommandSpec Move(string id, string label, string menu, string shortcut, string parameters, StepKind step) => new()
    {
        Id = id, Label = label, Menu = menu, Shortcut = shortcut, Params = parameters, Prompt = Prompt.SelectedObject, Mutates = Mutates.Scene,
        Run = (s, p) => ObjectStep(s, p, step),
    };

    /// <summary>One-step moves in the hierarchy, for keyboards and menus. The object keeps its world placement.</summary>
    private static JsonObject ObjectStep(Session session, JsonValue p, StepKind step)
    {
        var id = IdParam(p);
        var scene = session.Scene;
        if (!scene.Contains(id))
        {
            throw Stop.Failed($"no object with id {id}");
        }
        var parent = scene.Parent(id);
        var siblings = scene.Children(parent);
        var index = Math.Max(0, siblings.ToList().IndexOf(id));
        StableId? newParent;
        int? newIndex;
        switch (step)
        {
            case StepKind.Up:
                (newParent, newIndex) = index > 0 ? (parent, index - 1) : throw Stop.Failed("the object is already first");
                break;
            case StepKind.Down:
                (newParent, newIndex) = index + 1 < siblings.Count ? (parent, index + 1) : throw Stop.Failed("the object is already last");
                break;
            case StepKind.Indent:
                (newParent, newIndex) = index > 0 ? ((StableId?)siblings[index - 1], (int?)null) : throw Stop.Failed("there is no previous object to move into");
                break;
            default:
                var oldParent = parent ?? throw Stop.Failed("the object is already at the root");
                var grandparent = scene.Parent(oldParent);
                var at = scene.Children(grandparent).ToList().IndexOf(oldParent);
                (newParent, newIndex) = (grandparent, at >= 0 ? at + 1 : 0);
                break;
        }
        scene.ReparentKeepingWorld(id, newParent, newIndex).OrFail();
        return J.Obj(("parent", J.Id(newParent)));
    }

    private static Scripts ScriptsOf(Session session, StableId id)
    {
        if (!session.Scene.Contains(id))
        {
            throw Stop.Failed($"no object with id {id}");
        }
        return session.Scene.TryGet<Scripts>(id, out var scripts) ? scripts : Scripts.Initial;
    }

    private static List<ScriptRef> Copied(Scripts scripts) =>
        scripts.Items.Select(s => new ScriptRef(s.Module, new SortedDictionary<string, PropertyValue>(s.Properties, Utf8Order.Instance))).ToList();

    private static JsonObject ScriptAdd(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "module");
        var id = f.Required("id", JsonRead.Id);
        var module = f.Required("module", JsonRead.Str).Replace('\\', '/');
        var scripts = Copied(ScriptsOf(session, id));
        scripts.Add(new ScriptRef(module));
        session.Scene.Set(id, new Scripts(scripts)).OrFail();
        PrefabCommands.NoteOverride(session.Scene, id, "scripts");
        return J.Obj(("index", J.N(scripts.Count - 1)));
    }

    private static JsonObject ScriptRemove(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "index");
        var id = f.Required("id", JsonRead.Id);
        var index = f.Required("index", Params.Index);
        var scripts = Copied(ScriptsOf(session, id));
        if (index >= scripts.Count)
        {
            throw Stop.BadParams($"the object has {scripts.Count} scripts; there is no index {index}");
        }
        scripts.RemoveAt(index);
        if (scripts.Count == 0)
        {
            session.Scene.RemoveComponent(id, "scripts").OrFail();
        }
        else
        {
            session.Scene.Set(id, new Scripts(scripts)).OrFail();
        }
        PrefabCommands.NoteOverride(session.Scene, id, "scripts");
        return new JsonObject();
    }

    private static JsonObject ScriptSetProperty(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "index", "name", "value");
        var id = f.Required("id", JsonRead.Id);
        var index = f.Required("index", Params.Index);
        var name = f.Required("name", JsonRead.Str);
        if (!f.Has("value"))
        {
            throw new JsonShapeException("missing field `value`");
        }
        var value = f.Nullable("value", PropertyValue.Read);
        var scripts = Copied(ScriptsOf(session, id));
        if (index >= scripts.Count)
        {
            throw Stop.BadParams($"the object has {scripts.Count} scripts; there is no index {index}");
        }
        if (name.Length == 0 || Text.Bytes(name) > 64)
        {
            throw Stop.BadParams("a property name has 1 to 64 bytes");
        }
        if (value is not null)
        {
            scripts[index].Properties[name] = value;
        }
        else
        {
            scripts[index].Properties.Remove(name);
        }
        session.Scene.Set(id, new Scripts(scripts)).OrFail();
        PrefabCommands.NoteOverride(session.Scene, id, "scripts");
        return new JsonObject();
    }

    private static JsonObject Reparent(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "parent", "index", "keepWorld");
        var id = f.Required("id", JsonRead.Id);
        var parent = Params.OptionalId(f, "parent");
        int? index = f.NullableValue("index", Params.Index);
        var keepWorld = f.Optional("keepWorld", JsonRead.Bool, true);
        (keepWorld ? session.Scene.ReparentKeepingWorld(id, parent, index) : session.Scene.Reparent(id, parent, index)).OrFail();
        return new JsonObject();
    }

    private static (StableId Id, string Component) ComponentParams(JsonValue p)
    {
        var f = Params.Of(p, "id", "component");
        return (f.Required("id", JsonRead.Id), f.Required("component", JsonRead.Str));
    }

    private static JsonObject ComponentSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "component", "value");
        var id = f.Required("id", JsonRead.Id);
        var component = f.Required("component", JsonRead.Str);
        var value = f.Required("value", v => v.Canonical());
        session.Scene.SetComponent(id, component, value).OrFail();
        PrefabCommands.NoteOverride(session.Scene, id, component);
        return new JsonObject();
    }

    private static JsonObject ComponentAdd(Session session, JsonValue p)
    {
        var (id, component) = ComponentParams(p);
        session.Scene.AddComponent(id, component).OrFail();
        PrefabCommands.NoteOverride(session.Scene, id, component);
        return new JsonObject();
    }

    private static JsonObject ComponentRemove(Session session, JsonValue p)
    {
        var (id, component) = ComponentParams(p);
        session.Scene.RemoveComponent(id, component).OrFail();
        PrefabCommands.NoteOverride(session.Scene, id, component);
        return new JsonObject();
    }

    private static JsonObject ComponentReset(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "component", "script");
        var id = f.Required("id", JsonRead.Id);
        var component = f.Required("component", JsonRead.Str);
        int? script = f.NullableValue("script", Params.Index);
        if (component == "prefab")
        {
            throw Stop.BadParams("the prefab link has no defaults: use scene.prefab.revert or scene.component.remove");
        }
        if (script is not null && component != "scripts")
        {
            throw Stop.BadParams("script is an index into the \"scripts\" component");
        }
        var current = session.Scene.Component(id, component).OrFail();
        var fromPrefab = PrefabCommands.ResetValues(session, id, component, script);
        JsonValue value;
        if (script is not { } index)
        {
            value = fromPrefab is { } theirs ? theirs.Value : session.Scene.Registry.Find(component)?.DefaultValue() ?? JsonValue.Null;
        }
        else
        {
            var scripts = Copied(ScriptsOf(session, id));
            if (index >= scripts.Count)
            {
                throw Stop.BadParams($"the object has {scripts.Count} scripts; there is no index {index}");
            }
            var properties = new SortedDictionary<string, PropertyValue>(Utf8Order.Instance);
            if (fromPrefab is { } theirs && JsonRead.Try(() => ScriptRef.Read(theirs.Value)) is { IsOk: true } read)
            {
                properties = read.Value.Properties;
            }
            scripts[index] = new ScriptRef(scripts[index].Module, properties);
            value = new Scripts(scripts).Write().Canonical();
        }
        var changed = !value.Equals(current);
        if (changed)
        {
            session.Scene.SetComponent(id, component, value).OrFail();
        }
        if (fromPrefab is { } prefab)
        {
            // Back to the file's values: the copy follows the file again.
            PrefabCommands.ForgetOverride(session, id, component);
            return J.Obj(("from", J.S("prefab")), ("source", J.S(prefab.Source)));
        }
        if (changed)
        {
            PrefabCommands.NoteOverride(session.Scene, id, component);
        }
        return J.Obj(("from", J.S("defaults")));
    }
}
