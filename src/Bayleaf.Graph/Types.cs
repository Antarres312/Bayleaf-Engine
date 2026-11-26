using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Graph;

/// <summary>What a variable holds.</summary>
public enum VariableType
{
    Number,
    Vector,
    TrueFalse,

    /// <summary>A 32-bit whole number.</summary>
    WholeNumber,
    Text,

    /// <summary>An object of the scene; it starts as none and is set while the game plays (files: <c>object</c>).</summary>
    SceneObject,
    Color,
}

/// <summary>Whether a variable or property holds one value, a list of them (in order, by index from 0) or a map of them (under text keys, in the keys' order).</summary>
public enum Shape
{
    /// <summary>One value (files: <c>single</c>).</summary>
    One,
    List,
    Map,
}

public static class Shapes
{
    public static readonly IReadOnlyList<Shape> All = [Shape.One, Shape.List, Shape.Map];

    private static readonly string[] Names = ["single", "list", "map"];

    public static string Label(this Shape shape) => shape switch
    {
        Shape.One => "Single Value",
        Shape.List => "List",
        _ => "Map",
    };

    /// <summary>The word a description uses before a type code (<c>list:number</c>).</summary>
    public static string Code(this Shape shape) => Names[(int)shape];

    internal static Shape Read(JsonValue value) => (Shape)JsonRead.Variant(value, Names);
}

public static class VariableTypes
{
    /// <summary>Every type, in the order menus list them.</summary>
    public static readonly IReadOnlyList<VariableType> All =
        [VariableType.Number, VariableType.WholeNumber, VariableType.Vector, VariableType.TrueFalse, VariableType.Text, VariableType.SceneObject, VariableType.Color];

    /// <summary>The names files keep the types as, in declaration order.</summary>
    private static readonly string[] Names = ["number", "vector", "trueFalse", "wholeNumber", "text", "object", "color"];

    /// <summary>Most items a list or map starts with: a variable's start, a property's default.</summary>
    public const int MaxItems = 1024;

    /// <summary>Longest text value, in characters.</summary>
    public const int MaxText = 256;

    internal static string FileName(this VariableType type) => Names[(int)type];

    internal static VariableType Read(JsonValue value) => (VariableType)JsonRead.Variant(value, Names);

    /// <summary>The word nodes keep a pin's type as (<c>name:type</c>).</summary>
    public static string Code(this VariableType type) => type switch
    {
        VariableType.Number => "number",
        VariableType.Vector => "vector",
        VariableType.TrueFalse => "bool",
        VariableType.WholeNumber => "whole",
        VariableType.Text => "text",
        VariableType.SceneObject => "object",
        _ => "color",
    };

    public static VariableType? FromCode(string code)
    {
        foreach (var type in All)
        {
            if (type.Code() == code)
            {
                return type;
            }
        }
        return null;
    }

    /// <summary>English name shown to people.</summary>
    public static string Label(this VariableType type) => type switch
    {
        VariableType.Number => "Number",
        VariableType.Vector => "Vector",
        VariableType.TrueFalse => "True/False",
        VariableType.WholeNumber => "Whole Number",
        VariableType.Text => "Text",
        VariableType.SceneObject => "Object",
        _ => "Color",
    };

    /// <summary>What a new variable of this type starts at.</summary>
    public static JsonValue Zero(this VariableType type) => type switch
    {
        VariableType.Number => J.D(0),
        VariableType.Vector => new JsonArray([J.D(0), J.D(0), J.D(0)]),
        VariableType.TrueFalse => J.B(false),
        VariableType.WholeNumber => JsonNumber.Of(0L),
        VariableType.Text => J.S(""),
        VariableType.SceneObject => JsonValue.Null,
        _ => new JsonArray([J.D(1), J.D(1), J.D(1)]),
    };

    /// <summary>
    /// Whether <paramref name="value"/> fits this type: a finite number, three finite numbers (a
    /// vector or a colour), true or false, text of at most <see cref="MaxText"/> characters, or none for an object.
    /// </summary>
    public static bool Fits(this VariableType type, JsonValue value) => type switch
    {
        VariableType.Number => J.IsNumber(value),
        VariableType.Vector or VariableType.Color => J.IsTriple(value),
        VariableType.TrueFalse => value is JsonBool,
        VariableType.WholeNumber => J.AsI64(value) is >= int.MinValue and <= int.MaxValue,
        VariableType.Text => value is JsonString s && Bayleaf.Core.Text.Chars(s.Value) <= MaxText,
        _ => value.IsNull,
    };

    /// <summary>"List of Number", and so on: a list pin's name shown to people.</summary>
    public static string ListLabel(this VariableType type) => $"List of {type.Label()}";

    /// <summary>"Map from Text to Number", and so on: a map pin's name, its keys' type (<paramref name="key"/>) first.</summary>
    public static string MapLabel(this VariableType type, VariableType key) => $"Map from {key.Label()} to {type.Label()}";

    /// <summary>The pin type of this type in <paramref name="shape"/>: one value's, or a list's or map's of it.</summary>
    public static PinType Pin(this VariableType type, Shape shape, VariableType key) => shape switch
    {
        Shape.List => PinType.List(type),
        Shape.Map => PinType.Map(key, type),
        _ => type switch
        {
            VariableType.Number => PinType.Number,
            VariableType.Vector => PinType.Vec3,
            VariableType.TrueFalse => PinType.Bool,
            VariableType.WholeNumber => PinType.WholeNumber,
            VariableType.Text => PinType.Text,
            VariableType.SceneObject => PinType.SceneObject,
            _ => PinType.Color,
        },
    };

    /// <summary>The code of this type in <paramref name="shape"/>: <c>number</c>, <c>list:number</c>, or <c>map:whole:number</c>.</summary>
    public static string ShapedCode(this VariableType type, Shape shape, VariableType key) => shape switch
    {
        Shape.One => type.Code(),
        Shape.List => $"list:{type.Code()}",
        _ => $"map:{key.Code()}:{type.Code()}",
    };

    /// <summary>
    /// The type, shape and key type a code names: <c>number</c>, <c>list:number</c>,
    /// <c>map:whole:number</c>, or <c>map:number</c> for a map with text keys.
    /// </summary>
    public static (VariableType Type, Shape Shape, VariableType Key)? FromShapedCode(string code)
    {
        var parts = code.Split(':');
        return parts switch
        {
            [var one] when FromCode(one) is { } t => (t, Shape.One, VariableType.Text),
            ["list", var item] when FromCode(item) is { } t => (t, Shape.List, VariableType.Text),
            ["map", var item] when FromCode(item) is { } t => (t, Shape.Map, VariableType.Text),
            ["map", var key, var item] when FromCode(item) is { } t && FromCode(key) is { } k => (t, Shape.Map, k),
            _ => null,
        };
    }

    /// <summary>
    /// Whether <paramref name="key"/>, a map key as files keep it (always text), is a key of this
    /// type: text of at most <see cref="MaxText"/> characters, a finite number, a whole number,
    /// <c>true</c> or <c>false</c>, three numbers <c>x,y,z</c>, or an object's name.
    /// </summary>
    public static bool FitsKey(this VariableType type, string key)
    {
        static bool Number(string t) => J.ParseF32(t.Trim()) is { } v && float.IsFinite(v);
        return type switch
        {
            VariableType.Text or VariableType.SceneObject => Bayleaf.Core.Text.Chars(key) <= MaxText,
            VariableType.Number => Number(key),
            VariableType.WholeNumber => J.ParseI32(key.Trim()) is not null,
            VariableType.TrueFalse => key is "true" or "false",
            _ => key.Split(',') is { Length: 3 } parts && parts.All(Number),
        };
    }

    /// <summary>
    /// A map key of this type that no entry has yet (<paramref name="taken"/> says which are):
    /// <c>key</c>, <c>key2</c>… for text and objects, <c>0</c>, <c>1</c>… for numbers, <c>false</c>
    /// then <c>true</c>, <c>0,0,0</c>, <c>1,0,0</c>… for vectors.
    /// </summary>
    public static string? FreshKey(this VariableType type, Func<string, bool> taken)
    {
        IEnumerable<string> candidates = type switch
        {
            VariableType.Text or VariableType.SceneObject => Enumerable.Range(1, MaxItems + 1).Select(n => n == 1 ? "key" : JsonRead.Invariant($"key{n}")),
            VariableType.Number or VariableType.WholeNumber => Enumerable.Range(0, MaxItems + 1).Select(n => JsonRead.Invariant($"{n}")),
            VariableType.TrueFalse => ["false", "true"],
            _ => Enumerable.Range(0, MaxItems + 1).Select(n => JsonRead.Invariant($"{n},0,0")),
        };
        return candidates.FirstOrDefault(k => !taken(k));
    }

    /// <summary>What a new variable of this type and shape starts at: the type's zero, or an empty list or map.</summary>
    public static JsonValue ZeroOf(this VariableType type, Shape shape) => shape switch
    {
        Shape.One => type.Zero(),
        Shape.List => new JsonArray(),
        _ => new JsonObject(),
    };

    /// <summary>
    /// Whether <paramref name="value"/> fits this type in <paramref name="shape"/>: one value that
    /// fits, or a list (an array) or map (an object, its keys of type <paramref name="key"/>) of at
    /// most <see cref="MaxItems"/> of them.
    /// </summary>
    public static bool FitsShape(this VariableType type, Shape shape, VariableType key, JsonValue value) => shape switch
    {
        Shape.One => type.Fits(value),
        Shape.List => value is JsonArray items && items.Count <= MaxItems && items.All(v => type.Fits(v)),
        _ => value.Canonical() is JsonObject entries && entries.Count <= MaxItems && entries.All(e => key.FitsKey(e.Key) && type.Fits(e.Value)),
    };
}

/// <summary>The kinds of what a pin carries.</summary>
public enum PinKind
{
    /// <summary>Order of actions (visual scripts): "then do this".</summary>
    Flow,

    /// <summary>A number (files: <c>float</c>).</summary>
    Number,
    Vec3,
    Color,
    Bool,

    /// <summary>A number or a vector, decided by what is connected (math nodes).</summary>
    Any,

    /// <summary>An object in the scene (visual scripts), or none (files: <c>object</c>).</summary>
    SceneObject,

    /// <summary>Text (visual scripts).</summary>
    Text,

    /// <summary>A whole number (visual scripts): 32 bits, from -2147483648 to 2147483647 (files: <c>int</c>).</summary>
    WholeNumber,

    /// <summary>A script on an object (visual scripts): which one, on which object, or none.</summary>
    Component,

    /// <summary>A list of values of one type (visual scripts).</summary>
    List,

    /// <summary>A map of values of one type under keys of one type (visual scripts).</summary>
    Map,

    /// <summary>What plays on a skeleton (animation states): a clip, a mix or a layer.</summary>
    Motion,
}

/// <summary>What a pin carries: a kind, and for a list or map the type of its items (and a map's keys).</summary>
public readonly record struct PinType
{
    private PinType(PinKind kind, VariableType item, VariableType key)
    {
        Kind = kind;
        Item = item;
        Key = key;
    }

    public PinKind Kind { get; }

    /// <summary>A list's or map's items' type.</summary>
    public VariableType Item { get; }

    /// <summary>A map's keys' type.</summary>
    public VariableType Key { get; }

    public static readonly PinType Flow = new(PinKind.Flow, default, default);
    public static readonly PinType Number = new(PinKind.Number, default, default);
    public static readonly PinType Vec3 = new(PinKind.Vec3, default, default);
    public static readonly PinType Color = new(PinKind.Color, default, default);
    public static readonly PinType Bool = new(PinKind.Bool, default, default);
    public static readonly PinType Any = new(PinKind.Any, default, default);
    public static readonly PinType SceneObject = new(PinKind.SceneObject, default, default);
    public static readonly PinType Text = new(PinKind.Text, default, default);
    public static readonly PinType WholeNumber = new(PinKind.WholeNumber, default, default);
    public static readonly PinType Component = new(PinKind.Component, default, default);
    public static readonly PinType Motion = new(PinKind.Motion, default, default);

    public static PinType List(VariableType item) => new(PinKind.List, item, default);

    public static PinType Map(VariableType key, VariableType item) => new(PinKind.Map, item, key);

    private static readonly string[] Names = ["flow", "float", "vec3", "color", "bool", "any", "object", "text", "int", "component", "list", "map", "motion"];

    /// <summary>English name shown to people.</summary>
    public string Label => Kind switch
    {
        PinKind.Flow => "Flow",
        PinKind.Number => "Number",
        PinKind.Vec3 => "Vector",
        PinKind.Color => "Color",
        PinKind.Bool => "True/False",
        PinKind.Any => "Number or Vector",
        PinKind.SceneObject => "Object",
        PinKind.Text => "Text",
        PinKind.WholeNumber => "Whole Number",
        PinKind.Component => "Component",
        PinKind.List => Item.ListLabel(),
        PinKind.Map => Item.MapLabel(Key),
        _ => "Motion",
    };

    /// <summary>
    /// Whether an output of this type may feed an input of type <paramref name="to"/>. Numbers widen
    /// to vectors and colours; vectors and colours convert into each other. Whether an <c>Any</c>
    /// output really fits is checked when the graph is compiled.
    /// </summary>
    public bool ConnectsTo(PinType to)
    {
        var (a, b) = (Kind, to.Kind);
        if (a == PinKind.List && b == PinKind.List)
        {
            return Item == to.Item;
        }
        if (a == PinKind.Map && b == PinKind.Map)
        {
            return Key == to.Key && Item == to.Item;
        }
        if (a is PinKind.List or PinKind.Map || b is PinKind.List or PinKind.Map)
        {
            return false;
        }
        if (a == PinKind.Motion || b == PinKind.Motion)
        {
            return a == b;
        }
        if (a is PinKind.Flow or PinKind.Bool or PinKind.SceneObject or PinKind.Text or PinKind.WholeNumber or PinKind.Component && a == b)
        {
            return true;
        }
        if (a is PinKind.Flow or PinKind.Bool or PinKind.SceneObject or PinKind.Text or PinKind.Component || b is PinKind.Flow or PinKind.Bool or PinKind.SceneObject or PinKind.Text or PinKind.Component)
        {
            return false;
        }
        // A whole number widens to a number (and on to a vector); a number becomes whole only
        // through To Whole Number, which says how to round.
        if (a == PinKind.WholeNumber)
        {
            return true;
        }
        // Math on whole numbers stays whole; the compiler checks what an `Any` output carries.
        if (b == PinKind.WholeNumber)
        {
            return a == PinKind.Any;
        }
        return !(a is PinKind.Vec3 or PinKind.Color && b == PinKind.Number);
    }

    /// <summary>The type as files and descriptions write it: <c>"float"</c>, <c>{"list": "number"}</c>, <c>{"map": ["text", "number"]}</c>.</summary>
    public JsonValue ToJson() => Kind switch
    {
        PinKind.List => new JsonObject { { "list", J.S(Item.FileName()) } },
        PinKind.Map => new JsonObject { { "map", new JsonArray([J.S(Key.FileName()), J.S(Item.FileName())]) } },
        _ => J.S(Names[(int)Kind]),
    };

    public override string ToString() => Label;
}

/// <summary>Messages built as the Rust engine builds them.</summary>
internal static class Say
{
    public static string Debug(string text) => Bayleaf.Core.Text.Debug(text);

    public static string Join(IEnumerable<string> words) => string.Join(", ", words);

    public static bool Equal(string a, string b) => string.Equals(a, b, StringComparison.Ordinal);
}
