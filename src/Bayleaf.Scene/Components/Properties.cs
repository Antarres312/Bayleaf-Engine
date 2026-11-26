using System.Globalization;
using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Scene.Components;

/// <summary>What kind of value a script property holds: what the Inspector edits it with and how the script reads it.</summary>
public enum PropertyType
{
    Number,

    /// <summary>A 32-bit whole number.</summary>
    WholeNumber,
    TrueFalse,
    Vector,
    Color,

    /// <summary>Text up to <see cref="PropertyTypes.MaxText"/> bytes.</summary>
    Text,

    /// <summary>An object of the scene, by its name; empty for none.</summary>
    SceneObject,

    /// <summary>A prefab file (<c>.bprefab</c>), by its path in the project; empty for none.</summary>
    Prefab,

    /// <summary>A sound file, by its path in the project; empty for none.</summary>
    Sound,

    /// <summary>A material graph (<c>.bmaterial</c>), by its path in the project; empty for none.</summary>
    Material,

    /// <summary>A data file (<c>.bdata</c>), by its path in the project; empty for none.</summary>
    Data,
}

public static class PropertyTypes
{
    /// <summary>Longest text (and object name) a script property holds, in bytes.</summary>
    public const int MaxText = 1024;

    /// <summary>Longest path a file property holds, in bytes.</summary>
    public const int MaxPath = 512;

    /// <summary>Most items a list or map property holds in the scene.</summary>
    public const int MaxItems = 1024;

    /// <summary>The file extensions an Audio Source's clip and a sound property may have.</summary>
    public static readonly IReadOnlyList<string> SoundExtensions = ["wav", "ogg", "flac", "mp3", "aif", "aiff", "opus", "m4a", "aac"];

    public static readonly IReadOnlyList<PropertyType> All = Enum.GetValues<PropertyType>();

    /// <summary>The file extensions a file type's paths end in; empty for the other types.</summary>
    public static IReadOnlyList<string> Extensions(this PropertyType type) => type switch
    {
        PropertyType.Prefab => ["bprefab"],
        PropertyType.Sound => SoundExtensions,
        PropertyType.Material => ["bmaterial"],
        PropertyType.Data => [DataFile.Extension],
        _ => [],
    };

    /// <summary>Whether values of this type name a file of the project (a path, empty for none).</summary>
    public static bool IsFile(this PropertyType type) => type.Extensions().Count > 0;

    /// <summary>Whether the type's values are text: text, an object's name or a file's path.</summary>
    public static bool IsText(this PropertyType type) => type is PropertyType.Text or PropertyType.SceneObject || type.IsFile();

    /// <summary>English name shown to people.</summary>
    public static string Label(this PropertyType type) => type switch
    {
        PropertyType.Number => "Number",
        PropertyType.WholeNumber => "Whole Number",
        PropertyType.TrueFalse => "True/False",
        PropertyType.Vector => "Vector",
        PropertyType.Color => "Color",
        PropertyType.Text => "Text",
        PropertyType.SceneObject => "Object",
        PropertyType.Prefab => "Prefab",
        PropertyType.Sound => "Sound",
        PropertyType.Material => "Material",
        _ => "Data File",
    };

    /// <summary>The word scripts declare it with.</summary>
    public static string Code(this PropertyType type) => type switch
    {
        PropertyType.Number => "number",
        PropertyType.WholeNumber => "whole",
        PropertyType.TrueFalse => "bool",
        PropertyType.Vector => "vector",
        PropertyType.Color => "color",
        PropertyType.Text => "text",
        PropertyType.SceneObject => "object",
        PropertyType.Prefab => "prefab",
        PropertyType.Sound => "sound",
        PropertyType.Material => "material",
        _ => "data",
    };

    public static PropertyType? FromCode(string code) => All.Where(t => t.Code() == code).Select(t => (PropertyType?)t).FirstOrDefault();

    private static bool Number(string text) =>
        float.TryParse(text.Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out var v) && float.IsFinite(v);

    /// <summary>
    /// Whether <paramref name="key"/>, a map key as the scene keeps it (always text), is a key of this
    /// type: text of at most <see cref="MaxText"/> bytes, a finite number, a whole number, true or
    /// false, or three numbers <c>x,y,z</c>.
    /// </summary>
    public static bool FitsKey(this PropertyType type, string key) => type switch
    {
        PropertyType.Text or PropertyType.SceneObject => Text.Bytes(key) <= MaxText && !Text.HasNul(key),
        PropertyType.Number => Number(key),
        PropertyType.WholeNumber => int.TryParse(key.Trim(), NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out _),
        PropertyType.TrueFalse => key is "true" or "false",
        PropertyType.Vector or PropertyType.Color => key.Split(',') is { Length: 3 } parts && parts.All(Number),
        _ => key.Length > 0 && new PropertyValue.Text(key).Fits(type),
    };

    /// <summary>A map key of this type that no entry has yet: <c>key</c>, <c>key2</c>… for text, <c>0</c>, <c>1</c>… for numbers.</summary>
    public static string? FreshKey(this PropertyType type, Func<string, bool> taken)
    {
        IEnumerable<string> candidates = type switch
        {
            PropertyType.Prefab or PropertyType.Sound or PropertyType.Material or PropertyType.Data => [],
            PropertyType.Text or PropertyType.SceneObject => Enumerable.Range(1, MaxItems + 1).Select(n => n == 1 ? "key" : $"key{n}"),
            PropertyType.Number or PropertyType.WholeNumber => Enumerable.Range(0, MaxItems + 1).Select(n => n.ToString(CultureInfo.InvariantCulture)),
            PropertyType.TrueFalse => ["false", "true"],
            _ => Enumerable.Range(0, MaxItems + 1).Select(n => $"{n},0,0"),
        };
        return candidates.FirstOrDefault(k => !taken(k));
    }

    /// <summary>The value a property of this type has when nothing sets it.</summary>
    public static PropertyValue Zero(this PropertyType type) => type switch
    {
        PropertyType.Number or PropertyType.WholeNumber => new PropertyValue.Number(0),
        PropertyType.TrueFalse => new PropertyValue.Flag(false),
        PropertyType.Vector => new PropertyValue.Triple(Vector3.Zero),
        PropertyType.Color => new PropertyValue.Triple(Vector3.One),
        _ => new PropertyValue.Text(""),
    };

    /// <summary>The value a property of this type and shape has when nothing sets it: the type's zero, or an empty list or map.</summary>
    public static PropertyValue ZeroOf(this PropertyType type, PropertyShape shape) => shape.Kind switch
    {
        ShapeKind.One => type.Zero(),
        ShapeKind.List => new PropertyValue.List([]),
        _ => new PropertyValue.Map(new SortedDictionary<string, PropertyValue>(Utf8Order.Instance)),
    };
}

public enum ShapeKind
{
    One,
    List,
    Map,
}

/// <summary>Whether a script property holds one value, a list of them, or a map of them under keys of one type.</summary>
public readonly record struct PropertyShape(ShapeKind Kind, PropertyType KeyType)
{
    public static readonly PropertyShape One = new(ShapeKind.One, PropertyType.Text);
    public static readonly PropertyShape List = new(ShapeKind.List, PropertyType.Text);

    public static PropertyShape Map(PropertyType keyType) => new(ShapeKind.Map, keyType);

    /// <summary>The word a description puts before the type code (<c>list:number</c>); <c>single</c> for one value.</summary>
    public string Code => Kind switch
    {
        ShapeKind.One => "single",
        ShapeKind.List => "list",
        _ => "map",
    };

    /// <summary>The type and shape a type code names: <c>number</c>, <c>list:number</c>, <c>map:number</c> (text keys) or <c>map:whole:number</c>.</summary>
    public static (PropertyType Type, PropertyShape Shape)? Parse(string code)
    {
        var parts = code.Split(':');
        PropertyType? Type(string c) => PropertyTypes.FromCode(c);
        return parts switch
        {
            [var one] when Type(one) is { } t => (t, One),
            ["list", var item] when Type(item) is { } t => (t, List),
            ["map", var item] when Type(item) is { } t => (t, Map(PropertyType.Text)),
            ["map", var key, var item] when Type(item) is { } t && Type(key) is { } k => (t, Map(k)),
            _ => null,
        };
    }

    /// <summary>Every shape: one value, a list, and a map with keys of each type.</summary>
    public static IEnumerable<PropertyShape> All() => new[] { One, List }.Concat(PropertyTypes.All.Select(Map));

    public override string ToString() => Kind == ShapeKind.Map ? $"Map({KeyType})" : Kind.ToString();
}

/// <summary>
/// A script property's value, stored in the scene as plain JSON: a number, true/false, three numbers
/// (a vector or a colour) or text (text, or an object's name), or the items of a list or map property.
/// </summary>
public abstract record PropertyValue
{
    private PropertyValue()
    {
    }

    public sealed record Flag(bool Value) : PropertyValue;

    public sealed record Number(double Value) : PropertyValue;

    public sealed record Triple(Vector3 Value) : PropertyValue;

    public sealed record Text(string Value) : PropertyValue;

    /// <summary>A list property's items, saved as <c>{"list": [...]}</c>.</summary>
    public sealed record List(IReadOnlyList<PropertyValue> Items) : PropertyValue
    {
        public bool Equals(List? other) => other is not null && Items.SequenceEqual(other.Items);

        public override int GetHashCode() => Items.Count;
    }

    /// <summary>A map property's values by key, saved as <c>{"map": {...}}</c>.</summary>
    public sealed record Map(SortedDictionary<string, PropertyValue> Items) : PropertyValue
    {
        public bool Equals(Map? other) => other is not null && Items.Count == other.Items.Count && Items.Zip(other.Items).All(p => p.First.Key == p.Second.Key && p.First.Value.Equals(p.Second.Value));

        public override int GetHashCode() => Items.Count;
    }

    /// <summary>Reads a value as the scene holds one: the first of the forms above that it has.</summary>
    public static PropertyValue Read(JsonValue value)
    {
        switch (value)
        {
            case JsonBool b:
                return new Flag(b.Value);
            case JsonNumber n:
                return new Number(n.ToDouble());
            case JsonArray { Count: 3 } a when a.All(i => i is JsonNumber):
                return new Triple(new Vector3(JsonRead.F32(a[0]), JsonRead.F32(a[1]), JsonRead.F32(a[2])));
            case JsonString s:
                return new Text(s.Value);
            case JsonObject o:
                if (o.Find("list") is JsonArray items && TryAll(items, out var list))
                {
                    return new List(list);
                }
                if (o.Find("map") is JsonObject entries)
                {
                    var map = new SortedDictionary<string, PropertyValue>(Utf8Order.Instance);
                    var ok = true;
                    foreach (var (key, item) in entries.Entries)
                    {
                        var read = JsonRead.Try(() => Read(item));
                        ok &= read.IsOk;
                        if (read.IsOk)
                        {
                            map[key] = read.Value;
                        }
                    }
                    if (ok)
                    {
                        return new Map(map);
                    }
                }
                break;
        }
        throw new JsonShapeException("data did not match any variant of untagged enum PropertyValue");
    }

    private static bool TryAll(JsonArray items, out List<PropertyValue> values)
    {
        values = [];
        foreach (var item in items)
        {
            var read = JsonRead.Try(() => Read(item));
            if (read.IsError)
            {
                return false;
            }
            values.Add(read.Value);
        }
        return true;
    }

    /// <summary>The value as written directly (vectors in single precision).</summary>
    public JsonValue Write() => this switch
    {
        Flag f => J.B(f.Value),
        Number n => J.D(n.Value),
        Triple t => J.V3(t.Value),
        Text t => J.S(t.Value),
        List l => new JsonObject { { "list", new JsonArray(l.Items.Select(i => i.Write())) } },
        Map m => new JsonObject { { "map", new JsonObject(m.Items.Select(e => new KeyValuePair<string, JsonValue>(e.Key, e.Value.Write()))) } },
        _ => JsonValue.Null,
    };

    /// <summary>
    /// Whether the value can be a property of type <paramref name="type"/>: the right shape, finite
    /// numbers, a whole number in 32 bits, colours from 0 to 1, text not too long.
    /// </summary>
    public bool Fits(PropertyType type) => (type, this) switch
    {
        (PropertyType.Number, Number n) => double.IsFinite(n.Value) && Math.Abs(n.Value) <= float.MaxValue,
        (PropertyType.WholeNumber, Number n) => n.Value % 1 == 0 && J.Within(n.Value, int.MinValue, int.MaxValue),
        (PropertyType.TrueFalse, Flag) => true,
        (PropertyType.Vector, Triple t) => float.IsFinite(t.Value.X) && float.IsFinite(t.Value.Y) && float.IsFinite(t.Value.Z),
        (PropertyType.Color, Triple t) => J.Within(t.Value.X, 0f, 1f) && J.Within(t.Value.Y, 0f, 1f) && J.Within(t.Value.Z, 0f, 1f),
        (PropertyType.Text or PropertyType.SceneObject, Text t) => Core.Text.Bytes(t.Value) <= PropertyTypes.MaxText && !Core.Text.HasNul(t.Value),
        (var file, Text t) when file.IsFile() => t.Value.Length == 0
            || (Core.Text.Bytes(t.Value) <= PropertyTypes.MaxPath && !Core.Text.HasNul(t.Value) && file.Extensions().Any(e => Core.Text.AsciiLower(t.Value).EndsWith($".{e}", StringComparison.Ordinal))),
        _ => false,
    };

    /// <summary>
    /// Whether the value can be a property of type <paramref name="type"/> in <paramref name="shape"/>:
    /// one value that fits, or a list or map of at most <see cref="PropertyTypes.MaxItems"/> of them.
    /// </summary>
    public bool FitsShape(PropertyType type, PropertyShape shape) => (shape.Kind, this) switch
    {
        (ShapeKind.One, var value) => value.Fits(type),
        (ShapeKind.List, List l) => l.Items.Count <= PropertyTypes.MaxItems && l.Items.All(v => v.Fits(type)),
        (ShapeKind.Map, Map m) => m.Items.Count <= PropertyTypes.MaxItems && m.Items.All(e => shape.KeyType.FitsKey(e.Key) && e.Value.Fits(type)),
        _ => false,
    };

    /// <summary>A value of a type and shape as a script's description lists it: one value, or a plain array or object of them.</summary>
    public static PropertyValue? FromDescribed(JsonValue value, PropertyType type, PropertyShape shape)
    {
        PropertyValue? One(JsonValue v)
        {
            var read = JsonRead.Try(() => Read(v));
            return read.IsOk && read.Value.Fits(type) ? read.Value : null;
        }
        PropertyValue? made;
        switch (shape.Kind)
        {
            case ShapeKind.One:
                made = One(value);
                break;
            case ShapeKind.List:
                if (value is not JsonArray a)
                {
                    return null;
                }
                var items = a.Select(One).ToList();
                made = items.Contains(null) ? null : new List(items!);
                break;
            default:
                if (value is not JsonObject o)
                {
                    return null;
                }
                var map = new SortedDictionary<string, PropertyValue>(Utf8Order.Instance);
                foreach (var (key, item) in o.Entries)
                {
                    if (One(item) is not { } v)
                    {
                        return null;
                    }
                    map[key] = v;
                }
                made = new Map(map);
                break;
        }
        return made is not null && made.FitsShape(type, shape) ? made : null;
    }

    /// <summary>The type a value of this shape has when no script declares it.</summary>
    public PropertyType ShapeType() => this switch
    {
        Flag => PropertyType.TrueFalse,
        Number => PropertyType.Number,
        Triple => PropertyType.Vector,
        Text => PropertyType.Text,
        List l => l.Items.Count > 0 ? l.Items[0].ShapeType() : PropertyType.Number,
        Map m => m.Items.Count > 0 ? m.Items.First().Value.ShapeType() : PropertyType.Number,
        _ => PropertyType.Number,
    };

    /// <summary>The value as one number: a number, a whole number, or 1 and 0 for true and false.</summary>
    public float? AsNumber() => this switch
    {
        Number n => (float)n.Value,
        Flag f => f.Value ? 1f : 0f,
        _ => null,
    };

    /// <summary>The value as three numbers: one number in the first place, or a vector or colour.</summary>
    public Vector3? AsNumbers() => this is Triple t ? t.Value : AsNumber() is { } n ? new Vector3(n, 0, 0) : null;

    public string? AsText() => this is Text t ? t.Value : null;

    /// <summary>A value of a type from three numbers, as Set Component writes fields; null for text.</summary>
    public static PropertyValue? FromNumbers(PropertyType type, Vector3 numbers)
    {
        PropertyValue? value = type switch
        {
            PropertyType.Number => new Number(numbers.X),
            PropertyType.WholeNumber => new Number(Math.Clamp(Math.Round((double)numbers.X, MidpointRounding.AwayFromZero), int.MinValue, int.MaxValue)),
            PropertyType.TrueFalse => new Flag(numbers.X != 0),
            PropertyType.Vector => new Triple(numbers),
            PropertyType.Color => new Triple(Vector3.Clamp(numbers, Vector3.Zero, Vector3.One)),
            _ => null,
        };
        return value is not null && value.Fits(type) ? value : null;
    }
}

/// <summary>A property a script declares: its name, type, shape and default.</summary>
public sealed record DeclaredProperty(string Name, PropertyType Type, PropertyShape Shape, PropertyValue Default)
{
    /// <summary>Whether <paramref name="value"/> can be this property's: its type, in its shape.</summary>
    public bool Accepts(PropertyValue value) => value.FitsShape(Type, Shape);
}

/// <summary>One script on an object; see <c>docs/script-abi.md</c>.</summary>
public sealed record ScriptRef(string Module, SortedDictionary<string, PropertyValue> Properties)
{
    public ScriptRef(string module)
        : this(module, new SortedDictionary<string, PropertyValue>(Utf8Order.Instance))
    {
    }

    public bool Equals(ScriptRef? other) => other is not null && Module == other.Module && Properties.Count == other.Properties.Count
        && Properties.Zip(other.Properties).All(p => p.First.Key == p.Second.Key && p.First.Value.Equals(p.Second.Value));

    public override int GetHashCode() => StringComparer.Ordinal.GetHashCode(Module);

    public static ScriptRef Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ScriptRef", true, "module", "properties");
        return new(f.Required("module", JsonRead.Str), f.Optional("properties", v => JsonRead.Map(v, PropertyValue.Read), new SortedDictionary<string, PropertyValue>(Utf8Order.Instance)));
    }

    public JsonValue Write()
    {
        var o = new JsonObject { { "module", J.S(Module) } };
        if (Properties.Count > 0)
        {
            o.Add("properties", new JsonObject(Properties.Select(p => new KeyValuePair<string, JsonValue>(p.Key, p.Value.Write()))));
        }
        return o;
    }

    /// <summary>Display name: the module's file name without its extension, in title case (<c>spinner.wasm</c> → <c>Spinner</c>).</summary>
    public string DisplayName()
    {
        var file = Module[(Module.LastIndexOfAny(['/', '\\']) + 1)..];
        var dot = file.LastIndexOf('.');
        var stem = dot >= 0 ? file[..dot] : file;
        return string.Join(' ', stem.Split(['_', '-', ' '], StringSplitOptions.RemoveEmptyEntries).Select(w =>
        {
            var first = w.EnumerateRunes().First();
            return first.ToString().ToUpperInvariant() + w[first.Utf16SequenceLength..];
        }));
    }
}

/// <summary>The scripts on an object, in the order they run. The inspector shows each one as its own card.</summary>
public sealed record Scripts(IReadOnlyList<ScriptRef> Items) : ISceneComponent<Scripts>
{
    /// <summary>Most scripts one object may carry.</summary>
    public const int MaxScripts = 64;

    public static string Key => "scripts";

    public static string Label => "Scripts";

    public static bool Removable => true;

    public static bool Listed => false;

    public static Scripts Initial => new([]);

    public bool Equals(Scripts? other) => other is not null && Items.SequenceEqual(other.Items);

    public override int GetHashCode() => Items.Count;

    public static Scripts Read(JsonValue value) => new(JsonRead.List(value, ScriptRef.Read));

    public JsonValue Write() => new JsonArray(Items.Select(s => s.Write()));

    public Result<Unit, string> Validate()
    {
        if (Items.Count > MaxScripts)
        {
            return Checks.Fail($"an object may carry at most {MaxScripts} scripts");
        }
        foreach (var script in Items)
        {
            if (script.Module.Length == 0 || Core.Text.HasNul(script.Module))
            {
                return Checks.Fail("a script needs a module path without NUL characters");
            }
            if (script.Properties.Values.Any(v => !PropertyTypes.All.Any(t => PropertyShape.All().Any(s => v.FitsShape(t, s)))))
            {
                return Checks.Fail($"a script property must be a finite number, true or false, three finite numbers, text up to 1024 bytes, or a list or map of at most {PropertyTypes.MaxItems} of one of them");
            }
        }
        return Checks.Ok;
    }
}
