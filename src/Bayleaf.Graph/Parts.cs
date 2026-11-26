using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Graph;

public enum GraphErrorKind
{
    NoNode,
    NoComment,
    UnknownKind,
    NoPin,
    Refused,
    Format,
}

/// <summary>Why an edit or a file was refused. Messages are shown to the user.</summary>
public sealed record GraphError(GraphErrorKind Kind, string Message)
{
    public static GraphError NoNode(uint id) => new(GraphErrorKind.NoNode, JsonRead.Invariant($"there is no node {id}"));

    public static GraphError NoComment(uint id) => new(GraphErrorKind.NoComment, JsonRead.Invariant($"there is no comment {id}"));

    public static GraphError UnknownKind(string kind, string domain) => new(GraphErrorKind.UnknownKind, $"there is no node kind \"{kind}\" in {domain} graphs");

    public static GraphError NoPin(string node, string side, string pin) => new(GraphErrorKind.NoPin, $"{node} has no {side} \"{pin}\"");

    public static GraphError Refused(string message) => new(GraphErrorKind.Refused, message);

    public static GraphError Format(string message) => new(GraphErrorKind.Format, $"not a graph file: {message}");

    public override string ToString() => Message;
}

/// <summary>One end of a link.</summary>
public sealed record PinRef(uint Node, string Pin)
{
    internal static PinRef Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct PinRef", true, "node", "pin");
        return new(f.Required("node", JsonRead.U32), f.Required("pin", JsonRead.Str));
    }

    internal JsonValue Write() => new JsonObject { { "node", J.U(Node) }, { "pin", J.S(Pin) } };
}

/// <summary>A link from an output pin to an input pin.</summary>
public sealed record Link(PinRef From, PinRef To)
{
    public Link(uint from, string fromPin, uint to, string toPin)
        : this(new PinRef(from, fromPin), new PinRef(to, toPin))
    {
    }

    internal static Link Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Link", true, "from", "to");
        return new(f.Required("from", PinRef.Read), f.Required("to", PinRef.Read));
    }

    internal JsonValue Write() => new JsonObject { { "from", From.Write() }, { "to", To.Write() } };
}

public sealed class Node(uint id, string kind, Vector2 position)
{
    public uint Id { get; set; } = id;

    public string Kind { get; set; } = kind;

    /// <summary>Top-left corner on the canvas.</summary>
    public Vector2 Position { get; set; } = position;

    /// <summary>Settings and the values of inputs without a link, by name.</summary>
    public SortedDictionary<string, JsonValue> Values { get; private init; } = new(Utf8Order.Instance);

    public Node Clone() => new(Id, Kind, Position) { Values = new(Values, Utf8Order.Instance) };

    internal static Node Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Node", true, "id", "kind", "position", "values");
        var node = new Node(f.Required("id", JsonRead.U32), f.Required("kind", JsonRead.Str), f.Required("position", J.ReadV2));
        foreach (var (name, item) in f.Optional("values", v => JsonRead.Map(v, J.Untyped), new SortedDictionary<string, JsonValue>(Utf8Order.Instance)))
        {
            node.Values[name] = item;
        }
        return node;
    }

    internal JsonValue Write()
    {
        var o = new JsonObject { { "id", J.U(Id) }, { "kind", J.S(Kind) }, { "position", J.V2(Position) } };
        if (Values.Count > 0)
        {
            o.Add("values", new JsonObject(Values));
        }
        return o;
    }
}

/// <summary>A note on the canvas: a titled box drawn behind the nodes it groups. It does not change what the graph compiles to.</summary>
public sealed class Comment(uint id, string text, Vector2 position, Vector2 size)
{
    public uint Id { get; set; } = id;

    public string Text { get; set; } = text;

    /// <summary>Top-left corner on the canvas.</summary>
    public Vector2 Position { get; set; } = position;

    public Vector2 Size { get; set; } = size;

    public Comment Clone() => new(Id, Text, Position, Size);

    internal static Comment Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Comment", true, "id", "text", "position", "size");
        return new(f.Required("id", JsonRead.U32), f.Required("text", JsonRead.Str), f.Required("position", J.ReadV2), f.Required("size", J.ReadV2));
    }

    internal JsonValue Write() => new JsonObject { { "id", J.U(Id) }, { "text", J.S(Text) }, { "position", J.V2(Position) }, { "size", J.V2(Size) } };
}

/// <summary>Reads and writes the type, shape and key fields that variables, properties and params share.</summary>
internal static class Typed
{
    public static (VariableType Type, Shape Shape, VariableType Key) Read(JsonFields f) =>
        (f.Required("type", VariableTypes.Read), f.Optional("shape", Shapes.Read, Shape.One), f.Optional("key", VariableTypes.Read, VariableType.Text));

    public static void Write(JsonObject o, VariableType type, Shape shape, VariableType key)
    {
        o.Add("type", J.S(type.FileName()));
        if (shape != Shape.One)
        {
            o.Add("shape", J.S(shape.Code()));
        }
        if (key != VariableType.Text)
        {
            o.Add("key", J.S(key.FileName()));
        }
    }
}

/// <summary>A variable a visual script declares: its name, what it holds, and what it starts at.</summary>
public sealed class Variable(string name, VariableType type, Shape shape, VariableType key, JsonValue start)
{
    public string Name { get; set; } = name;

    public VariableType Type { get; set; } = type;

    /// <summary>One value, a list or a map.</summary>
    public Shape Shape { get; set; } = shape;

    /// <summary>A map's keys' type; text for others.</summary>
    public VariableType Key { get; set; } = key;

    public JsonValue Start { get; set; } = start.Canonical();

    public Variable Clone() => new(Name, Type, Shape, Key, Start);

    internal static Variable Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Variable", true, "name", "type", "shape", "key", "start");
        var name = f.Required("name", JsonRead.Str);
        var (type, shape, key) = Typed.Read(f);
        return new(name, type, shape, key, f.Required("start", J.Untyped));
    }

    internal JsonValue Write()
    {
        var o = new JsonObject { { "name", J.S(Name) } };
        Typed.Write(o, Type, Shape, Key);
        o.Add("start", Start);
        return o;
    }
}

/// <summary>
/// How a visual script's property shows in the Inspector, as a Rust script's layout says it (see
/// <c>docs/script-abi.md</c>): its label and hint, a group it goes in, a range and slider,
/// read-only, a list of choices, and a condition on another property.
/// </summary>
/// <param name="Group">The group it shows in, by title; properties of the same group show together.</param>
/// <param name="ShowIf">Shown only while the property named first has the value second.</param>
public sealed record PropertyOptions(
    string Label,
    string Hint,
    string Group,
    (double Low, double High)? Range,
    bool Slider,
    bool ReadOnly,
    IReadOnlyList<string> Choices,
    (string Property, JsonValue Value)? ShowIf)
{
    public static readonly PropertyOptions Empty = new("", "", "", null, false, false, [], null);

    public bool IsEmpty =>
        Label.Length == 0 && Hint.Length == 0 && Group.Length == 0 && Range is null && !Slider && !ReadOnly && Choices.Count == 0 && ShowIf is null;

    internal static PropertyOptions Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct PropertyOptions", true, "label", "hint", "group", "range", "slider", "readOnly", "choices", "showIf");
        return new(
            f.Optional("label", JsonRead.Str, ""),
            f.Optional("hint", JsonRead.Str, ""),
            f.Optional("group", JsonRead.Str, ""),
            f.NullableValue("range", v =>
            {
                var r = JsonRead.Fixed(v, 2, JsonRead.F64);
                return (r[0], r[1]);
            }),
            f.Optional("slider", JsonRead.Bool, false),
            f.Optional("readOnly", JsonRead.Bool, false),
            f.Optional("choices", v => (IReadOnlyList<string>)JsonRead.List(v, JsonRead.Str), []),
            f.NullableValue("showIf", ReadShowIf));
    }

    private static (string, JsonValue) ReadShowIf(JsonValue value)
    {
        if (value is not JsonArray a)
        {
            throw new JsonShapeException($"invalid type: {JsonRead.Describe(value)}, expected a tuple of size 2");
        }
        if (a.Count != 2)
        {
            throw new JsonShapeException(JsonRead.Invariant($"invalid length {a.Count}, expected a tuple of size 2"));
        }
        return (JsonRead.Str(a[0]), a[1].Canonical());
    }

    internal JsonValue Write()
    {
        var o = new JsonObject();
        if (Label.Length > 0)
        {
            o.Add("label", J.S(Label));
        }
        if (Hint.Length > 0)
        {
            o.Add("hint", J.S(Hint));
        }
        if (Group.Length > 0)
        {
            o.Add("group", J.S(Group));
        }
        if (Range is var (low, high))
        {
            o.Add("range", new JsonArray([J.D(low), J.D(high)]));
        }
        if (Slider)
        {
            o.Add("slider", J.B(true));
        }
        if (ReadOnly)
        {
            o.Add("readOnly", J.B(true));
        }
        if (Choices.Count > 0)
        {
            o.Add("choices", J.Strings(Choices));
        }
        if (ShowIf is var (property, shown))
        {
            o.Add("showIf", new JsonArray([J.S(property), shown.Canonical()]));
        }
        return o;
    }
}

/// <summary>
/// A property a visual script declares: a value the Inspector sets per object, its type and the
/// default it has where the object sets none. An object property's default is none (<c>null</c>).
/// </summary>
public sealed class DeclaredProperty(string name, VariableType type, Shape shape, VariableType key, JsonValue @default, PropertyOptions inspector)
{
    public string Name { get; set; } = name;

    public VariableType Type { get; set; } = type;

    public Shape Shape { get; set; } = shape;

    public VariableType Key { get; set; } = key;

    public JsonValue Default { get; set; } = @default.Canonical();

    /// <summary>How the Inspector shows it; empty for a plain row.</summary>
    public PropertyOptions Inspector { get; set; } = inspector;

    public DeclaredProperty Clone() => new(Name, Type, Shape, Key, Default, Inspector);

    internal static DeclaredProperty Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Property", true, "name", "type", "shape", "key", "default", "inspector");
        var name = f.Required("name", JsonRead.Str);
        var (type, shape, key) = Typed.Read(f);
        return new(name, type, shape, key, f.Required("default", J.Untyped), f.Optional("inspector", PropertyOptions.Read, PropertyOptions.Empty));
    }

    internal JsonValue Write()
    {
        var o = new JsonObject { { "name", J.S(Name) } };
        Typed.Write(o, Type, Shape, Key);
        o.Add("default", Default);
        if (!Inspector.IsEmpty)
        {
            o.Add("inspector", Inspector.Write());
        }
        return o;
    }
}

/// <summary>One input or output of a function, or a value an event brings: a name and what it holds.</summary>
public sealed record Param(string Name, VariableType Type, Shape Shape, VariableType Key)
{
    /// <summary>One value of <paramref name="type"/> named <paramref name="name"/>.</summary>
    public Param(string name, VariableType type)
        : this(name, type, Shape.One, VariableType.Text)
    {
    }

    internal static Param Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Param", true, "name", "type", "shape", "key");
        var name = f.Required("name", JsonRead.Str);
        var (type, shape, key) = Typed.Read(f);
        return new(name, type, shape, key);
    }

    internal JsonValue Write()
    {
        var o = new JsonObject { { "name", J.S(Name) } };
        Typed.Write(o, Type, Shape, Key);
        return o;
    }
}

/// <summary>
/// A function a visual script declares. Its Function node begins its actions with the inputs as
/// values, a Return node gives back the outputs, and Call Function nodes run it.
/// </summary>
public sealed record DeclaredFunction(string Name, IReadOnlyList<Param> Inputs, IReadOnlyList<Param> Outputs)
{
    internal static DeclaredFunction Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Function", true, "name", "inputs", "outputs");
        return new(f.Required("name", JsonRead.Str), f.Optional("inputs", ReadParams, []), f.Optional("outputs", ReadParams, []));
    }

    internal static IReadOnlyList<Param> ReadParams(JsonValue value) => JsonRead.List(value, Param.Read);

    internal JsonValue Write() =>
        new JsonObject { { "name", J.S(Name) }, { "inputs", new JsonArray(Inputs.Select(p => p.Write())) }, { "outputs", new JsonArray(Outputs.Select(p => p.Write())) } };
}

/// <summary>
/// An event a visual script declares: its name and what it brings. Its On Event nodes run when a
/// script, the interface or the engine triggers it on the script's object; Trigger Event nodes
/// trigger it, here or on another object.
/// </summary>
public sealed record DeclaredEvent(string Name, IReadOnlyList<Param> Params)
{
    internal static DeclaredEvent Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Event", true, "name", "params");
        return new(f.Required("name", JsonRead.Str), f.Optional("params", DeclaredFunction.ReadParams, []));
    }

    internal JsonValue Write() => new JsonObject { { "name", J.S(Name) }, { "params", new JsonArray(Params.Select(p => p.Write())) } };
}

/// <summary>Copied nodes and the links between them.</summary>
public sealed record Fragment(string Format, string Domain, IReadOnlyList<Node> Nodes, IReadOnlyList<Link> Links)
{
    /// <summary><c>format</c> of copied nodes on the clipboard.</summary>
    public const string FragmentFormat = "bayleaf.nodes";

    /// <summary>Reads copied nodes, as the clipboard holds them.</summary>
    public static Result<Fragment, string> FromJson(string text)
    {
        if (!JsonReader.Parse(text).TryGet(out var value, out var error))
        {
            return Result.Fail(error);
        }
        return JsonRead.Try(() =>
        {
            var f = JsonFields.Of(value, "struct Fragment", true, "format", "domain", "nodes", "links");
            return new Fragment(
                f.Required("format", JsonRead.Str),
                f.Required("domain", JsonRead.Str),
                f.Required("nodes", v => JsonRead.List(v, Node.Read)),
                f.Optional("links", v => (IReadOnlyList<Link>)JsonRead.List(v, Link.Read), []));
        });
    }

    public string ToJson() =>
        new JsonObject { { "format", J.S(Format) }, { "domain", J.S(Domain) }, { "nodes", new JsonArray(Nodes.Select(n => n.Write())) }, { "links", new JsonArray(Links.Select(l => l.Write())) } }
            .ToPretty();
}

/// <summary>What a paste added.</summary>
/// <param name="Ids">The new nodes.</param>
/// <param name="Skipped">Nodes left out because the graph may have only one of them (their labels).</param>
public sealed record Pasted(IReadOnlyList<uint> Ids, IReadOnlyList<string> Skipped);

/// <summary>One problem that stops a graph from compiling. Messages are shown to the user.</summary>
/// <param name="Node">The node the problem is on, if any.</param>
public sealed record Problem(uint? Node, string Message)
{
    public static Problem On(uint node, string message) => new(node, message);

    public static Problem OfGraph(string message) => new(null, message);

    /// <summary>Problems as one line.</summary>
    public static string Summary(IEnumerable<Problem> problems) => string.Join("; ", problems.Select(p => p.Message));

    public JsonValue ToJson()
    {
        var o = new JsonObject();
        if (Node is { } node)
        {
            o.Add("node", J.U(node));
        }
        o.Add("message", J.S(Message));
        return o;
    }
}
