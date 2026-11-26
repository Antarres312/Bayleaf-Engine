using System.Numerics;

using Bayleaf.Animation;
using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Graph;
using Bayleaf.Render;
using Bayleaf.Script;

using GraphModel = Bayleaf.Graph.Graph;

namespace Bayleaf.Engine;

/// <summary>
/// Graph editor commands: open, edit and save material graphs, visual scripts and animation states.
/// Several graphs stay open, each in a tab of its own; the commands work on the one in use. Edits
/// are undo steps in the same history as scene edits.
/// </summary>
/// <remarks>
/// The C# engine has no visual script compiler yet (visual scripts compile to C# with the script
/// host, docs/csharp-redesign.md stage 4), so a visual script's problems say that it is not compiled
/// rather than that nothing is wrong with it. Materials and animation states compile as in Rust.
/// </remarks>
internal static class GraphCommands
{
    /// <summary>Why graph edits are disabled with no graph open.</summary>
    public const string NoGraph = "open or create a graph first";

    /// <summary>What a visual script's problems say while the C# engine cannot compile one (AGENTS.md rule 15).</summary>
    public const string NotCompiled =
        "visual scripts are not compiled by the C# engine yet: they will compile to C# with the script host (docs/csharp-redesign.md, stage 4); until then the Rust engine (rust/) compiles them and finds their problems";

    /// <summary>Where pasted and duplicated nodes go, from the originals: down and to the right.</summary>
    private static readonly Vector2 DuplicateOffset = new(40f, 40f);

    /// <summary>The catalogs of every graph domain.</summary>
    private static readonly Catalog[] Catalogs = [Material.Catalog, Visual.Catalog, AnimationStates.Catalog];

    /// <summary>The node kinds of <paramref name="domain"/> (<c>material</c>, <c>script</c> or <c>animation</c>).</summary>
    public static Catalog? CatalogForDomain(string domain) => Catalogs.FirstOrDefault(c => c.Domain == domain);

    /// <summary>The domain whose files have <paramref name="path"/>'s extension.</summary>
    public static Catalog? CatalogForPath(string path) => ExtensionOf(path) is { } extension ? Catalogs.FirstOrDefault(c => c.Extension == extension) : null;

    /// <summary>The extension as Rust's <c>Path::extension</c> gives it: none without a dot or for a name starting with its only dot.</summary>
    internal static string? ExtensionOf(string path)
    {
        var name = Path.GetFileName(path.TrimEnd('/', '\\'));
        var dot = name.LastIndexOf('.');
        return name is "" or ".." || dot <= 0 ? null : name[(dot + 1)..];
    }

    /// <summary>What stops <paramref name="graph"/> from compiling, or nothing.</summary>
    public static IReadOnlyList<Problem> Problems(GraphModel graph)
    {
        switch (graph.Domain)
        {
            case "material":
                return Material.Compile(graph).TryGet(out _, out var material) ? [] : material;
            case "animation":
                return AnimationStates.Compile(graph).TryGet(out _, out var states) ? [] : states;
            case "script":
                return [Problem.OfGraph(NotCompiled)];
            default:
                return [Problem.OfGraph($"unknown graph domain {graph.Domain}")];
        }
    }

    /// <summary>A graph's undo step read back, as the open graph's catalog reads it.</summary>
    internal static Result<GraphModel, string> FromSnapshot(Session.Snapshot snapshot, Catalog? catalog) =>
        catalog is null ? Result.Fail("unknown graph domain") : GraphModel.FromJson(snapshot.Text, catalog).MapError(e => e.Message);

    private static string? HasGraph(Session session) => session.Graph is null ? NoGraph : null;

    private static CommandSpec Spec(string id, string label, string parameters, Mutates mutates, Func<Session, JsonValue, JsonValue> run) => new()
    {
        Id = id,
        Label = label,
        Params = parameters,
        Mutates = mutates,
        Enabled = id.StartsWith("graph.node.", StringComparison.Ordinal) || id.StartsWith("graph.link.", StringComparison.Ordinal)
            || id.StartsWith("graph.comment.", StringComparison.Ordinal) || id == "graph.save"
            ? HasGraph
            : Session.Always,
        Run = run,
    };

    public static IEnumerable<CommandSpec> All() =>
    [
        Spec("graph.catalog", "Graph Node Kinds", "{domain: \"material\" | \"script\" | \"animation\"}", Mutates.No, (_, p) => Domain(p).ToJson()),
        Spec("graph.new", "New Graph", "{domain: \"material\" | \"script\" | \"animation\"}", Mutates.No, New),
        Spec("graph.open", "Open Graph", "{path} (.bmaterial or .bscript)", Mutates.No, Open),
        Spec("graph.save", "Save Graph", "{path?} (needed for a graph that has no file yet)", Mutates.No, Save),
        Spec(
            "graph.close",
            "Close Graph",
            "{id?}: closes the graph in use (or the open graph id) with its undo steps; the one opened before it is then in use",
            Mutates.No,
            Close),
        Spec(
            "graph.get",
            "Show Graph",
            "none; returns the graph in use: {id, domain, path, dirty, graph, problems, outputTypes}; outputTypes says what each number-or-vector output gives now, by its inputs: [{node, pin, type}]",
            Mutates.No,
            Get),
        Spec(
            "graph.list",
            "Open Graphs",
            "none; the open graphs, each in a tab of its own: {graphs: [{id, domain, path, dirty, inUse}]}",
            Mutates.No,
            List),
        Spec("graph.activate", "Use Graph", "{id} or {path}: makes that open graph the one the graph.* commands work on", Mutates.No, Activate),
        Spec("graph.node.add", "Add Node", "{kind, position: [x, y]}", Mutates.Graph, NodeAdd),
        Spec("graph.node.remove", "Remove Node", "{node}", Mutates.Graph, NodeRemove),
        Spec("graph.node.move", "Move Node", "{node, position: [x, y]}", Mutates.Graph, NodeMove),
        Spec("graph.node.set", "Set Node Value", "{node, name, value} (null restores the default)", Mutates.Graph, NodeSet),
        Spec("graph.node.copy", "Copy Nodes", "{nodes: [id]}: returns {text}, the nodes and the links between them as clipboard text", Mutates.No, NodeCopy),
        Spec(
            "graph.node.paste",
            "Paste Nodes",
            "{text, offset?: [x, y]}: adds copies of copied nodes with new ids; returns {nodes, skipped}",
            Mutates.Graph,
            NodePaste),
        Spec(
            "graph.node.duplicate",
            "Duplicate Nodes",
            "{nodes: [id]}: adds copies beside them, linked as the originals are to each other; returns {nodes, skipped}",
            Mutates.Graph,
            NodeDuplicate),
        Spec("graph.link.add", "Link Pins", "{from: {node, pin}, to: {node, pin}}", Mutates.Graph, LinkAdd),
        Spec("graph.link.remove", "Unlink Pin", "{node, pin}: removes every link at that pin", Mutates.Graph, LinkRemove),
        Spec(
            "graph.comment.add",
            "Add Comment",
            "{position: [x, y], size?: [w, h], text?}: a box behind nodes, with a note; returns {comment}",
            Mutates.Graph,
            CommentAdd),
        Spec("graph.comment.set", "Edit Comment", "{comment, text?, position?, size?}", Mutates.Graph, CommentSet),
        Spec("graph.comment.remove", "Remove Comment", "{comment}: the nodes in it stay", Mutates.Graph, CommentRemove),
        Spec(
            "graph.variable.add",
            "Add Variable",
            "{name, type: \"number\" | \"wholeNumber\" | \"vector\" | \"trueFalse\" | \"text\" | \"object\" | \"color\", shape?, key?, start?}: declares a variable of a visual script, starting at start (else zero); shape \"list\" or \"map\" makes it a list (items in order, by index from 0) or a map (values under keys of type key, else text) of that type, starting empty or with start ([...] or {...})",
            Mutates.Graph,
            VariableAdd),
        Spec(
            "graph.variable.set",
            "Change Variable",
            "{name, rename?, type?, shape?, key?, start?}: a new start value, a new name that the variable's nodes follow, or a new type, shape (\"single\", \"list\", \"map\") or map key type (key); the start stays where it still fits, else it goes to zero, and its nodes follow the new type",
            Mutates.Graph,
            VariableSet),
        Spec("graph.variable.remove", "Remove Variable", "{name}: its nodes keep working, starting from zero", Mutates.Graph, VariableRemove),
        Spec(
            "graph.property.add",
            "Add Property",
            "{name, type, shape?, key?, default?}: declares a property of a visual script (type and shape as for variables), with a default (else zero); the Inspector sets it per object",
            Mutates.Graph,
            PropertyAdd),
        Spec(
            "graph.property.set",
            "Change Property",
            "{name, rename?, type?, shape?, key?, default?, inspector?}: a new default, a new type or shape (as for variables; options that no longer fit go), a new name that its Property and Set Property nodes follow, or how the Inspector shows it ({label?, hint?, group?, range?, slider?, readOnly?, choices?, showIf?}; {} for a plain row)",
            Mutates.Graph,
            PropertySet),
        Spec("graph.property.remove", "Remove Property", "{name}: its nodes stay, reading zero where no object sets it", Mutates.Graph, PropertyRemove),
        Spec(
            "graph.function.add",
            "Add Function",
            "{name}: declares a function of a visual script, with no inputs or outputs yet; its Function node begins its actions, Return gives back its outputs and Call Function runs it",
            Mutates.Graph,
            FunctionAdd),
        Spec(
            "graph.function.set",
            "Change Function",
            "{name, rename?, inputs?, outputs?}: a new name, or new inputs or outputs ([{name, type}], type as for variables); the function's nodes follow, and links to pins that went away go",
            Mutates.Graph,
            FunctionSet),
        Spec("graph.function.remove", "Remove Function", "{name}: its nodes stay, and say they name no function", Mutates.Graph, FunctionRemove),
        Spec(
            "graph.event.add",
            "Add Event",
            "{name}: declares an event of a visual script, bringing no values yet; its On Event nodes run when a script or the interface triggers it on the object, and Trigger Event nodes trigger it",
            Mutates.Graph,
            EventAdd),
        Spec(
            "graph.event.set",
            "Change Event",
            "{name, rename?, params?}: a new name, or new values ([{name, type}], type as for variables); the event's nodes follow, and links to pins that went away go",
            Mutates.Graph,
            EventSet),
        Spec("graph.event.remove", "Remove Event", "{name}: its nodes stay (a Trigger Event may name another script's event)", Mutates.Graph, EventRemove),
        Spec(
            "graph.menu.set",
            "Set Menu Path",
            "{path}: where Add Component lists the visual script (Light/Flicker); empty for under Scripts",
            Mutates.Graph,
            MenuSet),
    ];

    private static JsonObject Nothing => new();

    /// <summary>Stops with a graph's refusal as a failure.</summary>
    private static T OrFail<T>(this Result<T, GraphError> result) => result.IsOk ? result.Value : throw Stop.Failed(result.Error.Message);

    private static Vector2 Vec2(JsonValue value)
    {
        var a = JsonRead.Fixed(value, 2, JsonRead.F32);
        return new Vector2(a[0], a[1]);
    }

    private static ulong U64(JsonValue value) => value switch
    {
        JsonNumber n when n.AsUInt64() is { } whole => whole,
        JsonNumber { IsInteger: true } n => throw new JsonShapeException($"invalid value: integer `{n.Format()}`, expected u64"),
        _ => throw new JsonShapeException($"invalid type: {JsonRead.Describe(value)}, expected u64"),
    };

    private static JsonArray Ids(IEnumerable<uint> ids) => new JsonArray(ids.Select(id => (JsonValue)J.N(id)));

    /// <summary>The graph in use and its catalog, for an edit.</summary>
    private static (GraphDoc Doc, Catalog Catalog) Open(Session session)
    {
        var doc = session.Graph ?? throw Stop.Failed(NoGraph);
        var catalog = doc.Catalog ?? throw Stop.Failed("unknown graph domain");
        return (doc, catalog);
    }

    /// <summary>The graph in use, when it is a visual script: only scripts have variables.</summary>
    private static GraphDoc OpenScript(Session session)
    {
        var (doc, catalog) = Open(session);
        return catalog.Domain == Visual.Catalog.Domain ? doc : throw Stop.Failed("only visual scripts have variables, functions and a menu path");
    }

    private static Catalog Domain(JsonValue p)
    {
        var domain = Params.Of(p, "domain").Required("domain", JsonRead.Str);
        return CatalogForDomain(domain) ?? throw Stop.BadParams($"unknown domain \"{domain}\"; use \"material\", \"script\" or \"animation\"");
    }

    private static JsonValue New(Session session, JsonValue p)
    {
        var catalog = Domain(p);
        session.AddGraph(new GraphDoc(GraphModel.Create(catalog), null));
        return Get(session, JsonValue.Null);
    }

    /// <summary>One spelling per file, to tell whether a graph is open already.</summary>
    private static bool SameFile(string a, string b) => AssetCommands.Canonical(a) == AssetCommands.Canonical(b);

    private static JsonValue Open(Session session, JsonValue p)
    {
        var given = Params.Of(p, "path").Required("path", JsonRead.Str);
        var catalog = CatalogForPath(given) ?? throw Stop.BadParams("graph files end in .bmaterial or .bscript");
        var path = session.Allowed(given, Access.Read);
        // A graph open already is brought forward, with its changes.
        if (session.Graphs.FirstOrDefault(g => g.Path is { } open && SameFile(open, path)) is { } already)
        {
            session.ActivateGraph(already.Id);
            return Get(session, JsonValue.Null);
        }
        var text = Files.ReadText(path).MapError(e => $"cannot read {path}: {e}").OrFail();
        var graph = GraphModel.FromJson(text, catalog).MapError(e => $"{path}: {e.Message}").OrFail();
        session.AddGraph(new GraphDoc(graph, path));
        return Get(session, JsonValue.Null);
    }

    private static JsonValue Save(Session session, JsonValue p)
    {
        var given = Params.Of(p, "path").Nullable("path", JsonRead.Str);
        var doc = session.Graph ?? throw Stop.Failed(NoGraph);
        var catalog = doc.Catalog ?? throw Stop.Failed("unknown graph domain");
        var path = given ?? doc.Path ?? throw Stop.BadParams("this graph has no file yet; give a path");
        if (ExtensionOf(path) != catalog.Extension)
        {
            throw Stop.BadParams($"{catalog.Label} files end in .{catalog.Extension}");
        }
        path = session.Allowed(path, Access.Write);
        Files.Write(path, System.Text.Encoding.UTF8.GetBytes(doc.Graph.ToJson())).MapError(e => $"cannot write {path}: {e}").OrFail();
        doc.Path = path;
        doc.Dirty = false;
        return J.Obj(("path", J.S(path)));
    }

    private static JsonValue Close(Session session, JsonValue p)
    {
        var id = Params.Of(p, "id").NullableValue("id", U64);
        if (id is { } some)
        {
            session.ActivateGraph(some);
        }
        if (session.Graph is null)
        {
            throw Stop.Failed(NoGraph);
        }
        session.CloseGraph();
        return J.Obj(("inUse", session.Graph is { } next ? JsonNumber.Of(next.Id) : JsonValue.Null));
    }

    private static JsonValue List(Session session, JsonValue p)
    {
        Params.None(p);
        var inUse = session.Graph?.Id;
        var graphs = session.Graphs.Select(g => (JsonValue)J.Obj(
            ("id", JsonNumber.Of(g.Id)), ("domain", J.S(g.Graph.Domain)), ("path", J.S(g.Path)), ("dirty", J.B(g.Dirty)), ("inUse", J.B(g.Id == inUse))));
        return J.Obj(("graphs", new JsonArray(graphs)));
    }

    private static JsonValue Activate(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "path");
        var id = f.NullableValue("id", U64);
        var path = f.Nullable("path", JsonRead.Str);
        ulong chosen = (id, path) switch
        {
            ({ } some, null) => some,
            (null, { } file) => session.Graphs.FirstOrDefault(g => g.Path is { } open && SameFile(open, file))?.Id ?? throw Stop.BadParams($"{file} is not open"),
            _ => throw Stop.BadParams("give the graph's id or its path"),
        };
        session.ActivateGraph(chosen);
        return J.Obj(("id", JsonNumber.Of(chosen)));
    }

    private static JsonValue Get(Session session, JsonValue p)
    {
        Params.None(p);
        var doc = session.Graph ?? throw Stop.Failed(NoGraph);
        return J.Obj(
            ("id", JsonNumber.Of(doc.Id)),
            ("domain", J.S(doc.Graph.Domain)),
            ("path", J.S(doc.Path)),
            ("dirty", J.B(doc.Dirty)),
            ("graph", doc.Graph.ToValue()),
            ("problems", new JsonArray(Problems(doc.Graph).Select(problem => problem.ToJson()))),
            ("outputTypes", OutputTypes(doc.Graph)));
    }

    /// <summary>What each number-or-vector output gives now (a vector, a number or a whole number), as the graph editor shows it.</summary>
    private static JsonArray OutputTypes(GraphModel graph)
    {
        var output = new JsonArray();
        if (CatalogForDomain(graph.Domain) is not { } catalog)
        {
            return output;
        }
        foreach (var node in graph.Nodes)
        {
            if (catalog.DefOf(node) is not { } def)
            {
                continue;
            }
            foreach (var pin in def.Outputs.Where(o => o.Type == PinType.Any))
            {
                var type = graph.OutputType(catalog, new PinRef(node.Id, pin.Name));
                output.Add(J.Obj(("node", J.N(node.Id)), ("pin", J.S(pin.Name)), ("type", type.ToJson())));
            }
        }
        return output;
    }

    private static JsonValue NodeAdd(Session session, JsonValue p)
    {
        var f = Params.Of(p, "kind", "position");
        var kind = f.Required("kind", JsonRead.Str);
        var position = f.Optional("position", Vec2, Vector2.Zero);
        var (doc, catalog) = Open(session);
        return J.Obj(("node", J.N(doc.Graph.AddNode(catalog, kind, position).OrFail())));
    }

    private static JsonValue NodeRemove(Session session, JsonValue p)
    {
        var node = Params.Of(p, "node").Required("node", JsonRead.U32);
        Open(session).Doc.Graph.RemoveNode(node).OrFail();
        return Nothing;
    }

    private static JsonValue NodeMove(Session session, JsonValue p)
    {
        var f = Params.Of(p, "node", "position");
        var node = f.Required("node", JsonRead.U32);
        var position = f.Required("position", Vec2);
        Open(session).Doc.Graph.MoveNode(node, position).OrFail();
        return Nothing;
    }

    private static JsonValue NodeSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "node", "name", "value");
        var node = f.Required("node", JsonRead.U32);
        var name = f.Required("name", JsonRead.Str);
        var value = f.Required("value", v => v);
        var (doc, catalog) = Open(session);
        doc.Graph.SetValue(catalog, node, name, value).OrFail();
        return Nothing;
    }

    private static List<uint> NodeIds(JsonValue p) => Params.Of(p, "nodes").Required("nodes", v => JsonRead.List(v, JsonRead.U32));

    /// <summary>Refuses ids that are not nodes of the graph, so a typo does not copy nothing quietly.</summary>
    private static void Known(GraphModel graph, List<uint> ids)
    {
        foreach (var id in ids)
        {
            if (graph.FindNode(id) is null)
            {
                throw Stop.Failed($"there is no node {id}");
            }
        }
        if (ids.Count == 0)
        {
            throw Stop.BadParams("name at least one node");
        }
    }

    private static JsonValue NodeCopy(Session session, JsonValue p)
    {
        var ids = NodeIds(p);
        var graph = Open(session).Doc.Graph;
        Known(graph, ids);
        return J.Obj(("text", J.S(graph.Copy(ids).ToValue().ToCompact())));
    }

    private static JsonObject Pasted(Pasted pasted, string what)
    {
        if (pasted.Ids.Count == 0)
        {
            throw Stop.Failed($"nothing to {what}: a graph has only one {string.Join(", ", pasted.Skipped)}");
        }
        return J.Obj(("nodes", Ids(pasted.Ids)), ("skipped", J.Strings(pasted.Skipped)));
    }

    private static JsonValue NodePaste(Session session, JsonValue p)
    {
        var f = Params.Of(p, "text", "offset");
        var text = f.Required("text", JsonRead.Str);
        var offset = f.NullableValue("offset", Vec2) ?? DuplicateOffset;
        var fragment = Fragment.FromJson(text).MapError(e => $"the text is not copied nodes: {e}").OrBadParams();
        var (doc, catalog) = Open(session);
        return Pasted(doc.Graph.Paste(catalog, fragment, offset).OrFail(), "paste");
    }

    private static JsonValue NodeDuplicate(Session session, JsonValue p)
    {
        var ids = NodeIds(p);
        var (doc, catalog) = Open(session);
        Known(doc.Graph, ids);
        return Pasted(doc.Graph.Paste(catalog, doc.Graph.Copy(ids), DuplicateOffset).OrFail(), "duplicate");
    }

    private static JsonValue LinkAdd(Session session, JsonValue p)
    {
        var f = Params.Of(p, "from", "to");
        var link = new Link(f.Required("from", PinRef.Read), f.Required("to", PinRef.Read));
        var (doc, catalog) = Open(session);
        // A list or map node takes the type of the list linked into it (one undo step with the link).
        var trial = doc.Graph.Clone();
        if (catalog.Domain == Visual.Catalog.Domain && Visual.AdoptedItem(trial, link) is var (id, item, key))
        {
            trial.SetValue(catalog, id, "item", J.Strings([item.Code(), key.Code()])).OrFail();
            trial.DropUnfitLinks(catalog, id);
        }
        trial.Connect(catalog, link).OrFail();
        doc.Graph = trial;
        return Nothing;
    }

    private static JsonValue LinkRemove(Session session, JsonValue p)
    {
        var pin = PinRef.Read(p is JsonNull ? new JsonObject() : p);
        var removed = Open(session).Doc.Graph.Disconnect(pin);
        return removed == 0 ? throw Stop.Failed($"node {pin.Node} pin \"{pin.Pin}\" has no links") : J.Obj(("removed", J.N(removed)));
    }

    private static JsonValue CommentAdd(Session session, JsonValue p)
    {
        var f = Params.Of(p, "position", "size", "text");
        var position = f.Required("position", Vec2);
        var size = f.NullableValue("size", Vec2);
        var text = f.Nullable("text", JsonRead.Str);
        var id = Open(session).Doc.Graph.AddComment(text ?? "Comment", position, size ?? GraphModel.CommentSize).OrFail();
        return J.Obj(("comment", J.N(id)));
    }

    private static JsonValue CommentSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "comment", "text", "position", "size");
        var comment = f.Required("comment", JsonRead.U32);
        var text = f.Nullable("text", JsonRead.Str);
        var position = f.NullableValue("position", Vec2);
        var size = f.NullableValue("size", Vec2);
        if (text is null && position is null && size is null)
        {
            throw Stop.BadParams("give text, position or size");
        }
        Open(session).Doc.Graph.EditComment(comment, text, position, size).OrFail();
        return Nothing;
    }

    private static JsonValue CommentRemove(Session session, JsonValue p)
    {
        var comment = Params.Of(p, "comment").Required("comment", JsonRead.U32);
        Open(session).Doc.Graph.RemoveComment(comment).OrFail();
        return Nothing;
    }

    /// <summary>The node kinds that name a declaration by their <c>name</c> setting: its Get and Set nodes, and the list and map nodes.</summary>
    private static List<string> NamingKinds((string Get, string Set) kinds) => [kinds.Get, kinds.Set, .. Visual.CollectionKinds];

    /// <summary>
    /// After a declaration <paramref name="name"/> went from <paramref name="old"/> to a new type and
    /// shape: its Get and Set nodes become the new type's (<paramref name="kinds"/> gives a type's two),
    /// keeping their places and the links that still fit, and its list and map nodes take what it now
    /// holds and the type of a map's keys.
    /// </summary>
    private static void FollowRetype(GraphModel graph, string name, VariableType old, (VariableType Type, Shape Shape, VariableType Key) now, Func<VariableType, (string Get, string Set)> kinds)
    {
        var catalog = Visual.Catalog;
        if (old != now.Type)
        {
            var (before, after) = (kinds(old), kinds(now.Type));
            string[] from = [before.Get, before.Set];
            string[] to = [after.Get, after.Set];
            var named = graph.Nodes
                .Where(n => n.Values.GetValueOrDefault("name") is JsonString s && s.Value == name)
                .Select(n => (n.Id, Index: Array.IndexOf(from, n.Kind)))
                .Where(n => n.Index >= 0)
                .ToList();
            foreach (var (id, index) in named)
            {
                graph.ChangeKind(catalog, id, to[index]).OrFail();
            }
        }
        if (now.Shape != Shape.One)
        {
            graph.FollowItem(catalog, name, now.Type, now.Key, Visual.CollectionKinds).OrFail();
        }
    }

    private static JsonValue VariableAdd(Session session, JsonValue p)
    {
        var f = Params.Of(p, "name", "type", "shape", "key", "start");
        var name = f.Required("name", JsonRead.Str);
        var type = f.Required("type", VariableTypes.Read);
        var shape = f.NullableValue("shape", Shapes.Read);
        var key = f.NullableValue("key", VariableTypes.Read);
        var start = f.Nullable("start", v => v);
        var doc = OpenScript(session);
        var trial = doc.Graph.Clone();
        trial.AddVariable(name, type, null).OrFail();
        trial.RetypeVariable(name, type, shape ?? Shape.One, key ?? VariableType.Text).OrFail();
        if (start is not null)
        {
            trial.SetVariable(name, null, start, []).OrFail();
        }
        doc.Graph = trial;
        return Nothing;
    }

    private static JsonValue VariableSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "name", "rename", "type", "shape", "key", "start");
        var name = f.Required("name", JsonRead.Str);
        var rename = f.Nullable("rename", JsonRead.Str);
        var type = f.NullableValue("type", VariableTypes.Read);
        var shape = f.NullableValue("shape", Shapes.Read);
        var key = f.NullableValue("key", VariableTypes.Read);
        var start = f.Nullable("start", v => v);
        var doc = OpenScript(session);
        var variable = doc.Graph.FindVariable(name) ?? throw Stop.BadParams($"there is no variable {name}");
        var old = (variable.Type, variable.Shape, variable.Key);
        var now = (type ?? old.Type, shape ?? old.Shape, key ?? old.Key);
        var trial = doc.Graph.Clone();
        if (now != old)
        {
            trial.RetypeVariable(name, now.Item1, now.Item2, now.Item3).OrFail();
            FollowRetype(trial, name, old.Type, now, Visual.VariableKinds);
        }
        trial.SetVariable(name, rename, start, NamingKinds(Visual.VariableKinds(now.Item1))).OrFail();
        doc.Graph = trial;
        return Nothing;
    }

    private static string NameOnly(JsonValue p) => Params.Of(p, "name").Required("name", JsonRead.Str);

    private static JsonValue VariableRemove(Session session, JsonValue p)
    {
        var name = NameOnly(p);
        OpenScript(session).Graph.RemoveVariable(name).OrFail();
        return Nothing;
    }

    private static JsonValue PropertyAdd(Session session, JsonValue p)
    {
        var f = Params.Of(p, "name", "type", "shape", "key", "default");
        var name = f.Required("name", JsonRead.Str);
        var type = f.Required("type", VariableTypes.Read);
        var shape = f.NullableValue("shape", Shapes.Read);
        var key = f.NullableValue("key", VariableTypes.Read);
        var value = f.Nullable("default", v => v);
        var doc = OpenScript(session);
        var trial = doc.Graph.Clone();
        trial.AddProperty(name, type, null).OrFail();
        trial.RetypeProperty(name, type, shape ?? Shape.One, key ?? VariableType.Text).OrFail();
        if (value is not null)
        {
            trial.SetProperty(name, null, value, []).OrFail();
        }
        doc.Graph = trial;
        return Nothing;
    }

    private static JsonValue PropertySet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "name", "rename", "type", "shape", "key", "default", "inspector");
        var name = f.Required("name", JsonRead.Str);
        var rename = f.Nullable("rename", JsonRead.Str);
        var type = f.NullableValue("type", VariableTypes.Read);
        var shape = f.NullableValue("shape", Shapes.Read);
        var key = f.NullableValue("key", VariableTypes.Read);
        var value = f.Nullable("default", v => v);
        var inspector = f.Nullable("inspector", PropertyOptions.Read);
        var doc = OpenScript(session);
        var property = doc.Graph.FindProperty(name) ?? throw Stop.BadParams($"there is no property {name}");
        var old = (property.Type, property.Shape, property.Key);
        var now = (type ?? old.Type, shape ?? old.Shape, key ?? old.Key);
        var trial = doc.Graph.Clone();
        if (now != old)
        {
            trial.RetypeProperty(name, now.Item1, now.Item2, now.Item3).OrFail();
            FollowRetype(trial, name, old.Type, now, Visual.PropertyKinds);
            // Options of the old type that no longer fit (a range on text) go; its words stay.
            if (Visual.CheckInspector(trial).IsError && trial.FindProperty(name) is { } changed)
            {
                var o = changed.Inspector;
                trial.SetPropertyOptions(name, PropertyOptions.Empty with { Label = o.Label, Hint = o.Hint, Group = o.Group, ReadOnly = o.ReadOnly }).OrFail();
            }
        }
        if (inspector is not null)
        {
            trial.SetPropertyOptions(name, inspector).OrFail();
        }
        // Options are checked as the script host checks them, before anything changes.
        Visual.CheckInspector(trial).OrBadParams();
        trial.SetProperty(name, rename, value, NamingKinds(Visual.PropertyKinds(now.Item1))).OrFail();
        doc.Graph = trial;
        return Nothing;
    }

    private static JsonValue PropertyRemove(Session session, JsonValue p)
    {
        var name = NameOnly(p);
        OpenScript(session).Graph.RemoveProperty(name).OrFail();
        return Nothing;
    }

    private static JsonValue FunctionAdd(Session session, JsonValue p)
    {
        var name = NameOnly(p);
        OpenScript(session).Graph.AddFunction(name).OrFail();
        return Nothing;
    }

    private static List<Param> ParamList(JsonValue value) => JsonRead.List(value, Param.Read);

    private static JsonValue FunctionSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "name", "rename", "inputs", "outputs");
        var name = f.Required("name", JsonRead.Str);
        var rename = f.Nullable("rename", JsonRead.Str);
        var inputs = f.Nullable("inputs", ParamList);
        var outputs = f.Nullable("outputs", ParamList);
        OpenScript(session).Graph.SetFunction(Visual.Catalog, name, rename, inputs, outputs, Visual.FunctionKinds).OrFail();
        return Nothing;
    }

    private static JsonValue FunctionRemove(Session session, JsonValue p)
    {
        var name = NameOnly(p);
        OpenScript(session).Graph.RemoveFunction(name).OrFail();
        return Nothing;
    }

    private static JsonValue EventAdd(Session session, JsonValue p)
    {
        var name = NameOnly(p);
        OpenScript(session).Graph.AddEvent(name).OrFail();
        return Nothing;
    }

    private static JsonValue EventSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "name", "rename", "params");
        var name = f.Required("name", JsonRead.Str);
        var rename = f.Nullable("rename", JsonRead.Str);
        var parameters = f.Nullable("params", ParamList);
        OpenScript(session).Graph.SetEvent(Visual.Catalog, name, rename, parameters, Visual.EventKinds).OrFail();
        return Nothing;
    }

    private static JsonValue EventRemove(Session session, JsonValue p)
    {
        var name = NameOnly(p);
        OpenScript(session).Graph.RemoveEvent(name).OrFail();
        return Nothing;
    }

    private static JsonValue MenuSet(Session session, JsonValue p)
    {
        var given = Params.Of(p, "path").Required("path", JsonRead.Str);
        var path = given.Trim().Length == 0 ? "" : Visual.MenuPath(given).OrBadParams();
        OpenScript(session).Graph.Menu = path;
        return J.Obj(("path", J.S(path)));
    }
}
