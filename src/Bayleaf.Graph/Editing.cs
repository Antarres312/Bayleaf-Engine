using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Graph;

/// <summary>Nodes, links and comments: adding, changing and removing them, all checked.</summary>
public sealed partial class Graph
{
    public Comment? FindComment(uint id) => Comments.Find(c => c.Id == id);

    /// <summary>Adds a comment box and returns its id.</summary>
    public Result<uint, GraphError> AddComment(string text, Vector2 position, Vector2 size)
    {
        if (Comments.Count >= MaxComments)
        {
            return Result.Fail(GraphError.Refused(JsonRead.Invariant($"a graph may have at most {MaxComments} comments")));
        }
        var valid = CheckCommentText(text);
        if (valid.IsError)
        {
            return valid.AsFailure();
        }
        if (!ClampPosition(position).TryGet(out var place, out var badPlace))
        {
            return Result.Fail(badPlace);
        }
        if (!ClampSize(size).TryGet(out var box, out var badBox))
        {
            return Result.Fail(badBox);
        }
        var id = Comments.Count == 0 ? 1u : NextId(Comments.Max(c => c.Id));
        Comments.Add(new(id, text, place, box));
        return id;
    }

    /// <summary>Changes a comment's text, place or size; null keeps that part.</summary>
    public Result<Unit, GraphError> EditComment(uint id, string? text, Vector2? position, Vector2? size)
    {
        if (text is not null)
        {
            var valid = CheckCommentText(text);
            if (valid.IsError)
            {
                return valid;
            }
        }
        Vector2? place = null;
        if (position is { } p)
        {
            if (!ClampPosition(p).TryGet(out var clamped, out var bad))
            {
                return No(bad);
            }
            place = clamped;
        }
        Vector2? box = null;
        if (size is { } s)
        {
            if (!ClampSize(s).TryGet(out var clamped, out var bad))
            {
                return No(bad);
            }
            box = clamped;
        }
        if (FindComment(id) is not { } comment)
        {
            return No(GraphError.NoComment(id));
        }
        comment.Text = text ?? comment.Text;
        comment.Position = place ?? comment.Position;
        comment.Size = box ?? comment.Size;
        return Done;
    }

    /// <summary>Removes a comment box; the nodes in it stay.</summary>
    public Result<Unit, GraphError> RemoveComment(uint id) => Comments.RemoveAll(c => c.Id == id) > 0 ? Done : No(GraphError.NoComment(id));

    public Node? FindNode(uint id) => Nodes.Find(n => n.Id == id);

    /// <summary>The node and its kind.</summary>
    public Result<(Node Node, NodeDef Def), GraphError> NodeWithDef(Catalog catalog, uint id)
    {
        if (FindNode(id) is not { } node)
        {
            return Result.Fail(GraphError.NoNode(id));
        }
        return catalog.DefOf(node) is { } def ? (node, def) : Result.Fail(GraphError.UnknownKind(node.Kind, catalog.Domain));
    }

    /// <summary>Adds a node and returns its id.</summary>
    public Result<uint, GraphError> AddNode(Catalog catalog, string kind, Vector2 position)
    {
        if (catalog.Get(kind) is not { } def)
        {
            return Result.Fail(GraphError.UnknownKind(kind, catalog.Domain));
        }
        if (Nodes.Count >= MaxNodes)
        {
            return Result.Fail(GraphError.Refused(JsonRead.Invariant($"a graph may have at most {MaxNodes} nodes")));
        }
        if (def.Unique && Nodes.Any(n => n.Kind == kind))
        {
            return Result.Fail(GraphError.Refused($"a graph may have only one {def.Label} node"));
        }
        var id = Nodes.Count == 0 ? 1u : NextId(Nodes.Max(n => n.Id));
        if (!ClampPosition(position).TryGet(out var place, out var bad))
        {
            return Result.Fail(bad);
        }
        Nodes.Add(new(id, kind, place));
        return id;
    }

    /// <summary>One more than <paramref name="highest"/>, staying at the largest id.</summary>
    private static uint NextId(uint highest) => highest == uint.MaxValue ? highest : highest + 1;

    /// <summary>Removes a node and its links.</summary>
    public Result<Unit, GraphError> RemoveNode(uint id)
    {
        if (Nodes.RemoveAll(n => n.Id == id) == 0)
        {
            return No(GraphError.NoNode(id));
        }
        Links.RemoveAll(l => l.From.Node == id || l.To.Node == id);
        return Done;
    }

    public Result<Unit, GraphError> MoveNode(uint id, Vector2 position)
    {
        if (!ClampPosition(position).TryGet(out var place, out var bad))
        {
            return No(bad);
        }
        if (FindNode(id) is not { } node)
        {
            return No(GraphError.NoNode(id));
        }
        node.Position = place;
        return Done;
    }

    /// <summary>Sets a setting or the value of an input without a link; <c>null</c> restores the default.</summary>
    public Result<Unit, GraphError> SetValue(Catalog catalog, uint id, string name, JsonValue value)
    {
        if (!NodeWithDef(catalog, id).TryGet(out var found, out var error))
        {
            return No(error);
        }
        var (node, def) = found;
        if (value.IsNull)
        {
            if (def.Input(name) is null && def.Setting(name) is null)
            {
                return No(GraphError.NoPin(def.Label, "value", name));
            }
            node.Values.Remove(name);
            return Done;
        }
        var valid = CheckValue(def, name, value);
        if (valid.IsError)
        {
            return valid;
        }
        node.Values[name] = value.Canonical();
        DropDanglingLinks(catalog, id);
        return Done;
    }

    /// <summary>Removes the links of <paramref name="id"/> to pins its definition no longer has, or whose types no longer fit (a setting changed what a pin carries).</summary>
    public void DropUnfitLinks(Catalog catalog, uint id)
    {
        DropDanglingLinks(catalog, id);
        var touching = Links.Where(l => l.From.Node == id || l.To.Node == id).ToList();
        foreach (var link in touching)
        {
            var others = WithLinks(Links.Where(l => l != link).ToList());
            if (others.CheckLink(catalog, link, others).IsError)
            {
                Links.RemoveAll(l => l == link);
            }
        }
    }

    /// <summary>Removes the links of <paramref name="id"/> to pins its definition no longer has (a setting changed its pins).</summary>
    private void DropDanglingLinks(Catalog catalog, uint id)
    {
        if (FindNode(id) is not { } node || catalog.DefOf(node) is not { } def)
        {
            return;
        }
        Links.RemoveAll(l => !((l.From.Node != id || def.Output(l.From.Pin) is not null) && (l.To.Node != id || def.Input(l.To.Pin) is not null)));
    }

    /// <summary>Joins an output to an input. A data input keeps only the newest link, and so does a flow output.</summary>
    public Result<Unit, GraphError> Connect(Catalog catalog, Link link)
    {
        if (!PinTypes(catalog, link).TryGet(out var types, out var error))
        {
            return No(error);
        }
        var flow = types.From == PinType.Flow;
        var trial = Clone();
        // The new link replaces the one it supersedes.
        trial.Links.RemoveAll(l => !(l != link && (flow ? l.From != link.From : l.To != link.To)));
        var fits = trial.CheckLink(catalog, link, trial);
        if (fits.IsError)
        {
            return fits.AsFailure();
        }
        if (trial.Links.Count >= MaxLinks)
        {
            return No(GraphError.Refused(JsonRead.Invariant($"a graph may have at most {MaxLinks} links")));
        }
        trial.Links.Add(link);
        // What the new link brings may turn an output further on into a vector that its links no
        // longer fit.
        foreach (var later in trial.Links)
        {
            if (!trial.PinTypes(catalog, later).TryGet(out var pins, out var bad))
            {
                return No(bad);
            }
            var from = pins.From == PinType.Any ? trial.OutputType(catalog, later.From) : pins.From;
            if (!from.ConnectsTo(pins.To))
            {
                string Label(uint id) => trial.FindNode(id) is { } n && catalog.DefOf(n) is { } d ? d.Label : "a node";
                return No(GraphError.Format($"then {Label(later.From.Node)} would give {from.Label} to {Label(later.To.Node)}, which takes {pins.To.Label}"));
            }
        }
        Become(trial);
        return Done;
    }

    /// <summary>Removes every link at a pin.</summary>
    public int Disconnect(PinRef pin) => Links.RemoveAll(l => l.From == pin || l.To == pin);

    /// <summary>Nodes <paramref name="ids"/> and the links between them, for the clipboard. Links to nodes outside are left out.</summary>
    public Fragment Copy(IReadOnlyCollection<uint> ids) => new(
        Fragment.FragmentFormat,
        Domain,
        Nodes.Where(n => ids.Contains(n.Id)).Select(n => n.Clone()).ToList(),
        Links.Where(l => ids.Contains(l.From.Node) && ids.Contains(l.To.Node)).ToList());

    /// <summary>
    /// Adds copies of <paramref name="fragment"/>'s nodes, moved by <paramref name="offset"/>, with new
    /// ids and the links between them. A node that may appear only once and is already here is left
    /// out. All or nothing: on an error the graph is unchanged. Returns the new ids, in the fragment's order.
    /// </summary>
    public Result<Pasted, GraphError> Paste(Catalog catalog, Fragment fragment, Vector2 offset)
    {
        if (fragment.Format != Fragment.FragmentFormat)
        {
            return Result.Fail(GraphError.Format($"the clipboard does not hold {Fragment.FragmentFormat} nodes"));
        }
        if (fragment.Domain != Domain)
        {
            return Result.Fail(GraphError.Refused($"these are {fragment.Domain} nodes; they cannot go into a {Domain} graph"));
        }
        var graph = Clone();
        var ids = new Dictionary<uint, uint>();
        var skipped = new List<string>();
        foreach (var node in fragment.Nodes)
        {
            if (catalog.DefOf(node) is not { } def)
            {
                return Result.Fail(GraphError.UnknownKind(node.Kind, catalog.Domain));
            }
            if (def.Unique && graph.Nodes.Any(n => n.Kind == node.Kind))
            {
                skipped.Add(def.Label);
                continue;
            }
            if (!graph.AddNode(catalog, node.Kind, node.Position + offset).TryGet(out var id, out var error))
            {
                return Result.Fail(error);
            }
            foreach (var (name, value) in node.Values)
            {
                var set = graph.SetValue(catalog, id, name, value);
                if (set.IsError)
                {
                    return set.AsFailure();
                }
            }
            ids[node.Id] = id;
        }
        foreach (var link in fragment.Links)
        {
            if (ids.TryGetValue(link.From.Node, out var from) && ids.TryGetValue(link.To.Node, out var to))
            {
                var joined = graph.Connect(catalog, new Link(from, link.From.Pin, to, link.To.Pin));
                if (joined.IsError)
                {
                    return joined.AsFailure();
                }
            }
        }
        Become(graph);
        return new Pasted(fragment.Nodes.Where(n => ids.ContainsKey(n.Id)).Select(n => ids[n.Id]).ToList(), skipped);
    }

    /// <summary>The link into an input pin, if any (for flow inputs, the first one).</summary>
    public Link? LinkInto(uint node, string pin) => Links.Find(l => l.To.Node == node && l.To.Pin == pin);

    /// <summary>The links out of an output pin.</summary>
    public IEnumerable<Link> LinksFrom(uint node, string pin) => Links.Where(l => l.From.Node == node && l.From.Pin == pin);

    /// <summary>The value stored for <paramref name="name"/>, if any.</summary>
    public JsonValue? ValueOf(uint node, string name) => FindNode(node)?.Values.GetValueOrDefault(name);

    /// <summary>
    /// What an output pin gives. A number-or-vector output (<c>Any</c>) takes it from its node's
    /// inputs: a vector when one of its number-or-vector inputs is a vector (linked, or a vector
    /// typed in), else a whole number when every number input is whole (unless the kind is
    /// <see cref="Catalog.Fractional"/>), else a number. <c>Any</c> while nothing decides it (a loop of links).
    /// </summary>
    public PinType OutputType(Catalog catalog, PinRef pin) => ResolveOutput(catalog, pin, 0);

    private PinType ResolveOutput(Catalog catalog, PinRef pin, int depth)
    {
        if (FindNode(pin.Node) is not { } node || catalog.DefOf(node) is not { } def || def.Output(pin.Pin) is not { } output)
        {
            return PinType.Any;
        }
        if (output.Type != PinType.Any)
        {
            return output.Type;
        }
        if (depth > 64)
        {
            return PinType.Any;
        }
        var (vector, whole, numbers) = (false, !catalog.Fractional.Contains(def.Kind), false);
        foreach (var input in def.Inputs.Where(p => p.Type == PinType.Any || p.Type == PinType.Number || p.Type == PinType.WholeNumber))
        {
            numbers = true;
            PinType type;
            if (LinkInto(pin.Node, input.Name) is { } link)
            {
                type = ResolveOutput(catalog, link.From, depth + 1);
            }
            else
            {
                var value = ValueOf(pin.Node, input.Name);
                type = input.Type == PinType.WholeNumber ? PinType.WholeNumber
                    : input.Type == PinType.Any && value is not null && AsTriple(value) is not null ? PinType.Vec3
                    : input.Type == PinType.Any && value is null && input.Default is PinDefault.Vec3 or PinDefault.Color ? PinType.Vec3
                    : PinType.Number;
            }
            if ((type == PinType.Vec3 || type == PinType.Color) && input.Type == PinType.Any)
            {
                vector = true;
            }
            else if (type == PinType.Any)
            {
                return PinType.Any;
            }
            else if (type != PinType.WholeNumber)
            {
                whole = false;
            }
        }
        return (numbers, vector, whole) switch
        {
            (false, _, _) => PinType.Any,
            (true, true, _) => PinType.Vec3,
            (true, false, true) => PinType.WholeNumber,
            _ => PinType.Number,
        };
    }

    private Result<(PinType From, PinType To), GraphError> PinTypes(Catalog catalog, Link link)
    {
        if (!NodeWithDef(catalog, link.From.Node).TryGet(out var from, out var error) || !NodeWithDef(catalog, link.To.Node).TryGet(out var to, out error))
        {
            return Result.Fail(error);
        }
        if (from.Def.Output(link.From.Pin) is not { } output)
        {
            return Result.Fail(GraphError.NoPin(from.Def.Label, "output", link.From.Pin));
        }
        if (to.Def.Input(link.To.Pin) is not { } input)
        {
            return Result.Fail(GraphError.NoPin(to.Def.Label, "input", link.To.Pin));
        }
        return (output.Type, input.Type);
    }

    /// <summary>
    /// Checks one link against the graph without it. Clashes with existing links are refused.
    /// <paramref name="types"/>, when given, is the graph whose links decide what a number-or-vector
    /// output gives; without it such an output fits any number input.
    /// </summary>
    private Result<(PinType From, PinType To), GraphError> CheckLink(Catalog catalog, Link link, Graph? types)
    {
        if (!PinTypes(catalog, link).TryGet(out var pins, out var error))
        {
            return Result.Fail(error);
        }
        // A number-or-vector output links as what its inputs make it.
        var (from, to) = pins;
        if (types is not null && from == PinType.Any)
        {
            from = types.OutputType(catalog, link.From);
        }
        var fromDef = NodeWithDef(catalog, link.From.Node).Value.Def;
        var toDef = NodeWithDef(catalog, link.To.Node).Value.Def;
        if (link.From.Node == link.To.Node)
        {
            return Result.Fail(GraphError.Format("a node cannot be linked to itself"));
        }
        if (!from.ConnectsTo(to))
        {
            return Result.Fail(GraphError.Format(
                $"{fromDef.Label} · {PinLabel(fromDef.Outputs, link.From.Pin)} gives {from.Label}, but {toDef.Label} · {PinLabel(toDef.Inputs, link.To.Pin)} takes {to.Label}"));
        }
        if (Reaches(link.To.Node, link.From.Node))
        {
            return Result.Fail(GraphError.Format($"linking {fromDef.Label} to {toDef.Label} would make a loop"));
        }
        if (Links.Contains(link))
        {
            return Result.Fail(GraphError.Refused("these pins are already linked"));
        }
        if (from == PinType.Flow && Links.Any(l => l.From == link.From))
        {
            return Result.Fail(GraphError.Refused("a flow output leads to one node; remove its link first"));
        }
        if (from != PinType.Flow && Links.Any(l => l.To == link.To))
        {
            return Result.Fail(GraphError.Refused("this input already has a link; remove it first"));
        }
        return (from, to);
    }

    /// <summary>True when following links forward from <paramref name="start"/> arrives at <paramref name="goal"/>.</summary>
    public bool Reaches(uint start, uint goal)
    {
        var stack = new Stack<uint>([start]);
        var seen = new HashSet<uint>();
        while (stack.TryPop(out var id))
        {
            if (id == goal)
            {
                return true;
            }
            if (seen.Add(id))
            {
                foreach (var link in Links.Where(l => l.From.Node == id))
                {
                    stack.Push(link.To.Node);
                }
            }
        }
        return false;
    }

    private static string PinLabel(IReadOnlyList<PinDef> pins, string name) => pins.FirstOrDefault(p => p.Name == name)?.Label ?? name;

    private static Result<Vector2, GraphError> ClampPosition(Vector2 position) =>
        float.IsFinite(position.X) && float.IsFinite(position.Y)
            ? new Vector2(Math.Clamp(position.X, -MaxCoord, MaxCoord), Math.Clamp(position.Y, -MaxCoord, MaxCoord))
            : Result.Fail(GraphError.Refused("positions must be finite numbers"));

    private static Result<Unit, GraphError> CheckCommentText(string text) =>
        Text.Chars(text) > MaxCommentText ? No(GraphError.Refused(JsonRead.Invariant($"a comment may have at most {MaxCommentText} characters"))) : Done;

    /// <summary>A comment size, at least the smallest box.</summary>
    private static Result<Vector2, GraphError> ClampSize(Vector2 size) =>
        float.IsFinite(size.X) && float.IsFinite(size.Y)
            ? new Vector2(Math.Clamp(size.X, MinCommentSize.X, MaxCoord), Math.Clamp(size.Y, MinCommentSize.Y, MaxCoord))
            : Result.Fail(GraphError.Refused("sizes must be finite numbers"));

    /// <summary>Checks a value for a setting or an input pin of <paramref name="def"/>.</summary>
    private static Result<Unit, GraphError> CheckValue(NodeDef def, string name, JsonValue value)
    {
        Result<Unit, GraphError> Bad(string what) => No(GraphError.Refused($"{def.Label} · {name} must be {what}"));
        if (def.Setting(name) is { } setting)
        {
            return setting.Kind switch
            {
                SettingKind.Number => J.IsNumber(value) ? Done : Bad("a finite number"),
                SettingKind.Color or SettingKind.Vec3 => J.IsTriple(value) ? Done : Bad("three finite numbers"),
                SettingKind.Text => value is JsonString s && Text.Chars(s.Value) <= MaxText ? Done : Bad(JsonRead.Invariant($"text of at most {MaxText} characters")),
                SettingKind.Name => value is JsonString s && J.IsName(s.Value) ? Done : Bad("a name of letters, digits and _ (at most 64)"),
                SettingKind.Choice choice => value is JsonString s && choice.Words.Contains(s.Value) ? Done : Bad($"one of {Say.Join(choice.Words)}"),
                _ => value is JsonArray names && names.Count <= 64 && names.All(n => n is JsonString t && Text.Chars(t.Value) is >= 1 and <= 64 && !Text.HasControl(t.Value))
                    ? Done
                    : Bad("a list of up to 64 names of 1 to 64 characters"),
            };
        }
        if (def.Input(name) is not { } pin)
        {
            return No(GraphError.NoPin(def.Label, "input or setting", name));
        }
        return pin.Type.Kind switch
        {
            PinKind.Flow => No(GraphError.Refused($"{def.Label} · {name} is a flow input and holds no value")),
            PinKind.Number or PinKind.Any when J.IsNumber(value) => Done,
            PinKind.Any when J.IsTriple(value) => Done,
            PinKind.Number or PinKind.Any => Bad("a finite number"),
            PinKind.Vec3 or PinKind.Color => J.IsTriple(value) ? Done : Bad("three finite numbers"),
            PinKind.Bool => value is JsonBool ? Done : Bad("true or false"),
            PinKind.SceneObject => No(GraphError.Refused($"{def.Label} · {name} takes an object only from a link")),
            PinKind.Component => No(GraphError.Refused($"{def.Label} · {name} takes a component only from a link")),
            PinKind.List or PinKind.Map => No(GraphError.Refused($"{def.Label} · {name} takes a list or map only from a link")),
            PinKind.Motion => No(GraphError.Refused($"{def.Label} · {name} takes a motion only from a link")),
            PinKind.WholeNumber => J.AsI64(value) is >= int.MinValue and <= int.MaxValue ? Done : Bad("a whole number from -2147483648 to 2147483647"),
            _ => value is JsonString s && Text.Chars(s.Value) <= MaxText ? Done : Bad(JsonRead.Invariant($"text of at most {MaxText} characters")),
        };
    }

    /// <summary>Reads a number value.</summary>
    public static float? AsF32(JsonValue value) => J.AsF64(value) is { } v ? (float)v : null;

    /// <summary>Reads a three-number value.</summary>
    public static Vector3? AsTriple(JsonValue value) =>
        value is JsonArray { Count: 3 } a && AsF32(a[0]) is { } x && AsF32(a[1]) is { } y && AsF32(a[2]) is { } z ? new Vector3(x, y, z) : null;
}
