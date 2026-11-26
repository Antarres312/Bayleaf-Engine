using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Graph;

/// <summary>
/// A graph of one domain: nodes joined by links between typed pins. What a node kind means, which
/// pins it has and how a graph turns into code belong to a domain (materials, visual scripts,
/// animation states); this model only knows the shapes, given as a <see cref="Catalog"/>, and keeps
/// every graph valid: links join an output to a compatible input, a data input has at most one
/// link, a flow output has at most one link, and there are no cycles.
/// </summary>
public sealed partial class Graph
{
    /// <summary><c>format</c> field of graph files.</summary>
    public const string FileFormat = "bayleaf.graph";

    /// <summary>
    /// Current graph file version. Version 2 added declared <c>variables</c>; version 3 moved a visual
    /// script's Menu Path node into <c>menu</c>; version 4 added <c>functions</c>; version 5 added
    /// declared <c>properties</c>; version 6 added how each property shows in the Inspector; version 7
    /// added declared <c>events</c>; version 8 let variables and properties hold lists or maps
    /// (<c>shape</c>). Older files still read: a Menu Path node becomes <c>menu</c>, and a catalog's
    /// upgrade moves what its own nodes kept.
    /// </summary>
    public const uint CurrentVersion = 8;

    /// <summary>The node kind versions before 3 kept a visual script's menu path in, as its <c>path</c> value.</summary>
    private const string OldMenuNode = "menuPath";

    /// <summary>Longest <c>menu</c>, in characters.</summary>
    public const int MaxMenu = 256;

    /// <summary>Most nodes in one graph.</summary>
    public const int MaxNodes = 2000;

    /// <summary>Most links in one graph.</summary>
    public const int MaxLinks = 8000;

    /// <summary>Longest text value, in characters.</summary>
    public const int MaxText = VariableTypes.MaxText;

    /// <summary>Positions are kept inside ±this many canvas units.</summary>
    public const float MaxCoord = 1.0e6f;

    /// <summary>Most comments in one graph.</summary>
    public const int MaxComments = 500;

    /// <summary>Most characters in one comment.</summary>
    public const int MaxCommentText = 2000;

    /// <summary>Smallest comment box, in canvas units.</summary>
    public static readonly Vector2 MinCommentSize = new(120f, 60f);

    /// <summary>Size of a new comment box when none is given.</summary>
    public static readonly Vector2 CommentSize = new(400f, 240f);

    /// <summary>Most properties a graph declares.</summary>
    public const int MaxProperties = 256;

    /// <summary>Most events a graph declares.</summary>
    public const int MaxEvents = 64;

    /// <summary>Most functions a graph declares.</summary>
    public const int MaxFunctions = 64;

    /// <summary>Most inputs or outputs a function has.</summary>
    public const int MaxParams = 16;

    /// <summary>Most variables a graph declares.</summary>
    public const int MaxVariables = 256;

    /// <summary>Pin names a function's inputs and outputs cannot take: the flow pins of its nodes.</summary>
    private static readonly string[] FlowPins = ["run", "then"];

    private Graph(string format, uint version, string domain)
    {
        Format = format;
        Version = version;
        Domain = domain;
    }

    public string Format { get; set; }

    public uint Version { get; set; }

    public string Domain { get; set; }

    public List<Node> Nodes { get; private set; } = [];

    public List<Link> Links { get; private set; } = [];

    public List<Comment> Comments { get; private set; } = [];

    /// <summary>Variables a visual script declares, with what they start at.</summary>
    public List<Variable> Variables { get; private set; } = [];

    /// <summary>Where Add Component lists a visual script (<c>Light/Flicker</c>); empty for under Scripts.</summary>
    public string Menu { get; set; } = "";

    /// <summary>Functions a visual script declares: what each takes and gives back.</summary>
    public List<DeclaredFunction> Functions { get; private set; } = [];

    /// <summary>Events a visual script declares: what each brings. Any script may trigger them on an object.</summary>
    public List<DeclaredEvent> Events { get; private set; } = [];

    /// <summary>Properties a visual script declares: values the Inspector sets per object, with defaults.</summary>
    public List<DeclaredProperty> Properties { get; private set; } = [];

    /// <summary>An empty graph of <paramref name="catalog"/>'s domain.</summary>
    public static Graph Empty(Catalog catalog) => new(FileFormat, CurrentVersion, catalog.Domain);

    /// <summary>A new graph with the catalog's starter nodes.</summary>
    public static Graph Create(Catalog catalog)
    {
        var graph = Empty(catalog);
        foreach (var (kind, position) in catalog.Starter)
        {
            // Starter kinds come from the catalog itself.
            _ = graph.AddNode(catalog, kind, position);
        }
        return graph;
    }

    /// <summary>A copy that shares nothing with this graph.</summary>
    public Graph Clone() => new(Format, Version, Domain)
    {
        Nodes = [.. Nodes.Select(n => n.Clone())],
        Links = [.. Links],
        Comments = [.. Comments.Select(c => c.Clone())],
        Variables = [.. Variables.Select(v => v.Clone())],
        Menu = Menu,
        Functions = [.. Functions],
        Events = [.. Events],
        Properties = [.. Properties.Select(p => p.Clone())],
    };

    /// <summary>This graph with other links, sharing the rest (only to read).</summary>
    private Graph WithLinks(List<Link> links) => new(Format, Version, Domain)
    {
        Nodes = Nodes,
        Links = links,
        Comments = Comments,
        Variables = Variables,
        Menu = Menu,
        Functions = Functions,
        Events = Events,
        Properties = Properties,
    };

    /// <summary>Takes everything of <paramref name="other"/>, as a trial edit that worked becomes the graph.</summary>
    private void Become(Graph other)
    {
        (Format, Version, Domain, Menu) = (other.Format, other.Version, other.Domain, other.Menu);
        (Nodes, Links, Comments, Variables) = (other.Nodes, other.Links, other.Comments, other.Variables);
        (Functions, Events, Properties) = (other.Functions, other.Events, other.Properties);
    }

    private static readonly Result<Unit, GraphError> Done = Result.Ok<GraphError>();

    private static Result<Unit, GraphError> No(GraphError error) => Result.Fail(error);

    /// <summary>Parses and checks a graph file of <paramref name="catalog"/>'s domain.</summary>
    public static Result<Graph, GraphError> FromJson(string text, Catalog catalog)
    {
        if (!Parse(text).TryGet(out var graph, out var error))
        {
            return Result.Fail(error);
        }
        var read = graph.Version;
        graph.Upgrade();
        if (read < CurrentVersion)
        {
            catalog.Upgrade(graph, read);
            graph.Version = CurrentVersion;
        }
        var checkedGraph = graph.Check(catalog);
        return checkedGraph.IsOk ? graph : checkedGraph.AsFailure();
    }

    /// <summary>The file's fields, read without any check.</summary>
    internal static Result<Graph, GraphError> Parse(string text)
    {
        var read = JsonReader.Parse(text).TryGet(out var value, out var parseError) ? JsonRead.Try(() => Read(value)) : Result.Fail(parseError);
        return read.MapError(GraphError.Format);
    }

    private static Graph Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Graph", true, "format", "version", "domain", "nodes", "links", "comments", "variables", "menu", "functions", "events", "properties");
        return new Graph(f.Required("format", JsonRead.Str), f.Required("version", JsonRead.U32), f.Required("domain", JsonRead.Str))
        {
            Nodes = f.Required("nodes", v => JsonRead.List(v, Node.Read)),
            Links = f.Optional("links", v => JsonRead.List(v, Link.Read), []),
            Comments = f.Optional("comments", v => JsonRead.List(v, Comment.Read), []),
            Variables = f.Optional("variables", v => JsonRead.List(v, Variable.Read), []),
            Menu = f.Optional("menu", JsonRead.Str, ""),
            Functions = f.Optional("functions", v => JsonRead.List(v, DeclaredFunction.Read), []),
            Events = f.Optional("events", v => JsonRead.List(v, DeclaredEvent.Read), []),
            Properties = f.Optional("properties", v => JsonRead.List(v, DeclaredProperty.Read), []),
        };
    }

    /// <summary>Moves what older versions kept elsewhere into its current place: a Menu Path node (before version 3) becomes <c>menu</c>.</summary>
    internal void Upgrade()
    {
        if (Version >= 3)
        {
            return;
        }
        var old = Nodes.Where(n => n.Kind == OldMenuNode).Select(n => n.Id).ToList();
        foreach (var id in old)
        {
            if (Menu.Length == 0 && ValueOf(id, "path") is JsonString path)
            {
                Menu = path.Value;
            }
        }
        Nodes.RemoveAll(n => old.Contains(n.Id));
        Links.RemoveAll(l => old.Contains(l.From.Node) || old.Contains(l.To.Node));
        Version = CurrentVersion;
    }

    /// <summary>The file's text, as the engine saves it.</summary>
    public string ToJson() => ToValue().ToPretty();

    /// <summary>The graph as a JSON value, with the fields the file has (numbers in single precision, as written).</summary>
    public JsonObject ToValue()
    {
        var o = new JsonObject
        {
            { "format", J.S(Format) },
            { "version", J.U(Version) },
            { "domain", J.S(Domain) },
            { "nodes", new JsonArray(Nodes.Select(n => n.Write())) },
            { "links", new JsonArray(Links.Select(l => l.Write())) },
        };
        if (Comments.Count > 0)
        {
            o.Add("comments", new JsonArray(Comments.Select(c => c.Write())));
        }
        if (Variables.Count > 0)
        {
            o.Add("variables", new JsonArray(Variables.Select(v => v.Write())));
        }
        if (Menu.Length > 0)
        {
            o.Add("menu", J.S(Menu));
        }
        if (Functions.Count > 0)
        {
            o.Add("functions", new JsonArray(Functions.Select(f => f.Write())));
        }
        if (Events.Count > 0)
        {
            o.Add("events", new JsonArray(Events.Select(e => e.Write())));
        }
        if (Properties.Count > 0)
        {
            o.Add("properties", new JsonArray(Properties.Select(p => p.Write())));
        }
        return o;
    }

    /// <summary>Checks a whole graph, as loaded from a file or an agent.</summary>
    public Result<Unit, GraphError> Check(Catalog catalog)
    {
        if (Format != FileFormat)
        {
            return No(GraphError.Format($"format must be \"{FileFormat}\""));
        }
        if (Version is < 1 or > CurrentVersion)
        {
            return No(GraphError.Format(JsonRead.Invariant($"graph version {Version} is not supported (this engine reads versions 1 to {CurrentVersion})")));
        }
        if (Text.Chars(Menu) > MaxMenu)
        {
            return No(GraphError.Format(JsonRead.Invariant($"the menu path is at most {MaxMenu} characters")));
        }
        if (Variables.Count > MaxVariables)
        {
            return No(GraphError.Format(JsonRead.Invariant($"a graph declares at most {MaxVariables} variables")));
        }
        for (var index = 0; index < Variables.Count; index++)
        {
            var variable = Variables[index];
            if (CheckVariableName(variable.Name).TryGet(out _, out var bad) is false)
            {
                return No(GraphError.Format(bad));
            }
            if (!variable.Type.FitsShape(variable.Shape, variable.Key, variable.Start))
            {
                return No(GraphError.Format($"variable {variable.Name} starts at a value that is not a {variable.Type.Label()}"));
            }
            if (Variables.Take(index).Any(v => v.Name == variable.Name))
            {
                return No(GraphError.Format($"variable {variable.Name} is declared twice"));
            }
        }
        if (Properties.Count > MaxProperties)
        {
            return No(GraphError.Format(JsonRead.Invariant($"a graph declares at most {MaxProperties} properties")));
        }
        for (var index = 0; index < Properties.Count; index++)
        {
            var property = Properties[index];
            if (CheckVariableName(property.Name).TryGet(out _, out var bad) is false)
            {
                return No(GraphError.Format(bad));
            }
            if (!property.Type.FitsShape(property.Shape, property.Key, property.Default))
            {
                return No(GraphError.Format($"property {property.Name} defaults to a value that is not a {property.Type.Label()}"));
            }
            if (Properties.Take(index).Any(p => p.Name == property.Name))
            {
                return No(GraphError.Format($"property {property.Name} is declared twice"));
            }
        }
        if (Functions.Count > MaxFunctions)
        {
            return No(GraphError.Format(JsonRead.Invariant($"a graph declares at most {MaxFunctions} functions")));
        }
        for (var index = 0; index < Functions.Count; index++)
        {
            var function = Functions[index];
            var bad = FirstError(() => CheckVariableName(function.Name), () => CheckParams(function.Name, function.Inputs), () => CheckParams(function.Name, function.Outputs));
            if (bad.IsError)
            {
                return No(GraphError.Format(bad.Error));
            }
            if (Functions.Take(index).Any(f => f.Name == function.Name))
            {
                return No(GraphError.Format($"function {function.Name} is declared twice"));
            }
        }
        if (Events.Count > MaxEvents)
        {
            return No(GraphError.Format(JsonRead.Invariant($"a graph declares at most {MaxEvents} events")));
        }
        for (var index = 0; index < Events.Count; index++)
        {
            var e = Events[index];
            var bad = FirstError(() => CheckVariableName(e.Name), () => CheckParams(e.Name, e.Params));
            if (bad.IsError)
            {
                return No(GraphError.Format(bad.Error));
            }
            if (Events.Take(index).Any(other => other.Name == e.Name))
            {
                return No(GraphError.Format($"event {e.Name} is declared twice"));
            }
        }
        if (Domain != catalog.Domain)
        {
            return No(GraphError.Format($"this is a {Domain} graph, not a {catalog.Domain} graph"));
        }
        if (Nodes.Count > MaxNodes || Links.Count > MaxLinks)
        {
            return No(GraphError.Format(JsonRead.Invariant($"a graph may have at most {MaxNodes} nodes and {MaxLinks} links")));
        }
        var ids = new HashSet<uint>();
        foreach (var node in Nodes)
        {
            if (!ids.Add(node.Id))
            {
                return No(GraphError.Format(JsonRead.Invariant($"node id {node.Id} is used twice")));
            }
            if (catalog.DefOf(node) is not { } def)
            {
                return No(GraphError.UnknownKind(node.Kind, catalog.Domain));
            }
            if (def.Unique && Nodes.Count(n => n.Kind == node.Kind) > 1)
            {
                return No(GraphError.Refused($"a graph may have only one {def.Label} node"));
            }
            if (!Within(node.Position))
            {
                return No(GraphError.Format(JsonRead.Invariant($"node {node.Id} has an invalid position")));
            }
            foreach (var (name, value) in node.Values)
            {
                var valid = CheckValue(def, name, value);
                if (valid.IsError)
                {
                    return valid;
                }
            }
        }
        if (Comments.Count > MaxComments)
        {
            return No(GraphError.Format(JsonRead.Invariant($"a graph may have at most {MaxComments} comments")));
        }
        var commentIds = new HashSet<uint>();
        foreach (var comment in Comments)
        {
            if (!commentIds.Add(comment.Id))
            {
                return No(GraphError.Format(JsonRead.Invariant($"comment id {comment.Id} is used twice")));
            }
            var text = CheckCommentText(comment.Text);
            if (text.IsError)
            {
                return text;
            }
            var bigEnough = comment.Size.X >= MinCommentSize.X && comment.Size.Y >= MinCommentSize.Y;
            if (!Within(comment.Position) || !Within(comment.Size) || !bigEnough)
            {
                return No(GraphError.Format(JsonRead.Invariant($"comment {comment.Id} has an invalid position or size")));
            }
        }
        var seen = WithLinks([]);
        foreach (var link in Links)
        {
            // A file is read by the pins' own types, so a graph saved with a link that no longer
            // fits still opens (its compiler says what is wrong, on the node, to mend there).
            var fits = seen.CheckLink(catalog, link, null);
            if (fits.IsError)
            {
                return fits.AsFailure();
            }
            seen.Links.Add(link);
        }
        return Done;
    }

    private static Result<Unit, string> FirstError(params Func<Result<Unit, string>>[] checks)
    {
        foreach (var check in checks)
        {
            var result = check();
            if (result.IsError)
            {
                return result;
            }
        }
        return Result.Ok<string>();
    }

    private static bool Within(Vector2 v) => float.IsFinite(v.X) && float.IsFinite(v.Y) && Math.Abs(v.X) <= MaxCoord && Math.Abs(v.Y) <= MaxCoord;
}
