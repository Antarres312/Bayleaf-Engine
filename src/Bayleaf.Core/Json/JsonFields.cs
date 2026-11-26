using System.Globalization;

namespace Bayleaf.Core.Json;

/// <summary>
/// A JSON value that does not have the shape its reader expects. Readers throw it; the public
/// boundary that reads a file or a command's value catches it and returns the message as an error.
/// </summary>
public sealed class JsonShapeException : Exception
{
    public JsonShapeException()
    {
    }

    public JsonShapeException(string message)
        : base(message)
    {
    }

    public JsonShapeException(string message, Exception inner)
        : base(message, inner)
    {
    }
}

/// <summary>
/// Reads an object's fields for a typed value, as the Rust engine's typed readers do: each field
/// once, required fields present, and (when the type says so) no fields it does not know.
/// </summary>
public sealed class JsonFields
{
    private readonly Dictionary<string, JsonValue> fields = new(StringComparer.Ordinal);
    private readonly List<string> order = [];
    private readonly HashSet<string> taken = new(StringComparer.Ordinal);
    private readonly bool denyUnknown;
    private readonly string[] known;

    private JsonFields(JsonObject obj, bool denyUnknown, string[] known)
    {
        this.denyUnknown = denyUnknown;
        this.known = known;
        foreach (var (key, value) in obj.Entries)
        {
            if (denyUnknown && !known.Contains(key, StringComparer.Ordinal))
            {
                throw new JsonShapeException($"unknown field `{key}`, {Expected(known)}");
            }
            if (!fields.TryAdd(key, value))
            {
                if (known.Contains(key, StringComparer.Ordinal))
                {
                    throw new JsonShapeException($"duplicate field `{key}`");
                }
                fields[key] = value;
            }
            order.Add(key);
        }
    }

    private static string Expected(string[] known) => known.Length switch
    {
        0 => "there are no fields",
        1 => $"expected `{known[0]}`",
        _ => $"expected one of {string.Join(", ", known.Select(k => $"`{k}`"))}",
    };

    /// <summary>The fields of <paramref name="value"/>, which must be an object of <paramref name="what"/>.</summary>
    public static JsonFields Of(JsonValue value, string what, bool denyUnknown, params string[] known)
    {
        if (value is not JsonObject obj)
        {
            throw new JsonShapeException($"invalid type: {JsonRead.Describe(value)}, expected {what}");
        }
        return new JsonFields(obj, denyUnknown, known);
    }

    /// <summary>Whether the object has <paramref name="name"/>, without reading it.</summary>
    public bool Has(string name) => fields.ContainsKey(name);

    public T Required<T>(string name, Func<JsonValue, T> read)
    {
        if (!fields.TryGetValue(name, out var value))
        {
            throw new JsonShapeException($"missing field `{name}`");
        }
        return Read(name, value, read);
    }

    /// <summary>A field that falls back to <paramref name="fallback"/> when it is missing (not when it is null).</summary>
    public T Optional<T>(string name, Func<JsonValue, T> read, T fallback) =>
        fields.TryGetValue(name, out var value) ? Read(name, value, read) : fallback;

    /// <summary>A field that may be missing or null.</summary>
    public T? Nullable<T>(string name, Func<JsonValue, T> read)
        where T : class =>
        fields.TryGetValue(name, out var value) && !value.IsNull ? Read(name, value, read) : null;

    /// <summary>A field of a value type that may be missing or null.</summary>
    public T? NullableValue<T>(string name, Func<JsonValue, T> read)
        where T : struct =>
        fields.TryGetValue(name, out var value) && !value.IsNull ? Read(name, value, read) : null;

    private T Read<T>(string name, JsonValue value, Func<JsonValue, T> read)
    {
        taken.Add(name);
        return read(value);
    }

    /// <summary>The names given that this reader knows nothing of, in order (for types that keep them).</summary>
    public IEnumerable<string> Unknown => order.Where(k => !known.Contains(k, StringComparer.Ordinal)).Distinct(StringComparer.Ordinal);

    /// <summary>For readers that keep the shape strict: true when unknown fields are refused.</summary>
    public bool DeniesUnknown => denyUnknown;
}

/// <summary>Readers of single values, failing with the Rust engine's kind of message.</summary>
public static class JsonRead
{
    /// <summary>How a message names a value of the wrong type.</summary>
    public static string Describe(JsonValue value) => value switch
    {
        JsonNull => "null",
        JsonBool b => $"boolean `{(b.Value ? "true" : "false")}`",
        JsonNumber { NumberKind: NumberKind.Natural or NumberKind.NegativeWhole } n => $"integer `{n.Format()}`",
        JsonNumber n => $"floating point `{n.Format()}`",
        JsonString s => $"string {Text.Debug(s.Value)}",
        JsonArray => "sequence",
        _ => "map",
    };

    private static JsonShapeException Wrong(JsonValue value, string expected) => new($"invalid type: {Describe(value)}, expected {expected}");

    public static bool Bool(JsonValue value) => value is JsonBool b ? b.Value : throw Wrong(value, "a boolean");

    public static string Str(JsonValue value) => value is JsonString s ? s.Value : throw Wrong(value, "a string");

    /// <summary>Any number, as single precision (as the Rust engine narrows a double).</summary>
    public static float F32(JsonValue value) => value is JsonNumber n ? n.ToSingle() : throw Wrong(value, "f32");

    public static double F64(JsonValue value) => value is JsonNumber n ? n.ToDouble() : throw Wrong(value, "f64");

    public static uint U32(JsonValue value)
    {
        if (value is JsonNumber n && n.IsInteger)
        {
            var whole = n.AsInt64();
            if (whole is >= 0 and <= uint.MaxValue)
            {
                return (uint)whole.Value;
            }
            throw new JsonShapeException($"invalid value: integer `{n.Format()}`, expected u32");
        }
        throw Wrong(value, "u32");
    }

    public static int I32(JsonValue value)
    {
        if (value is JsonNumber n && n.IsInteger)
        {
            var whole = n.AsInt64();
            if (whole is >= int.MinValue and <= int.MaxValue)
            {
                return (int)whole.Value;
            }
            throw new JsonShapeException($"invalid value: integer `{n.Format()}`, expected i32");
        }
        throw Wrong(value, "i32");
    }

    public static StableId Id(JsonValue value)
    {
        var text = Str(value);
        var parsed = StableId.ParseHex(text);
        return parsed.IsOk ? parsed.Value : throw new JsonShapeException(parsed.Error);
    }

    /// <summary>An array of exactly <paramref name="length"/> items.</summary>
    public static T[] Fixed<T>(JsonValue value, int length, Func<JsonValue, T> read)
    {
        if (value is not JsonArray a)
        {
            throw Wrong(value, $"an array of length {length}");
        }
        if (a.Count != length)
        {
            throw new JsonShapeException($"invalid length {a.Count}, expected an array of length {length}");
        }
        return a.Select(read).ToArray();
    }

    public static List<T> List<T>(JsonValue value, Func<JsonValue, T> read) => value is JsonArray a ? a.Select(read).ToList() : throw Wrong(value, "a sequence");

    /// <summary>An object's values by key, sorted as the Rust engine keeps a map (a later duplicate winning).</summary>
    public static SortedDictionary<string, T> Map<T>(JsonValue value, Func<JsonValue, T> read)
    {
        if (value is not JsonObject o)
        {
            throw Wrong(value, "a map");
        }
        var map = new SortedDictionary<string, T>(Utf8Order.Instance);
        foreach (var (key, item) in o.Entries)
        {
            map[key] = read(item);
        }
        return map;
    }

    /// <summary>One of <paramref name="names"/>, by its place in the list.</summary>
    public static int Variant(JsonValue value, params string[] names)
    {
        if (value is not JsonString s)
        {
            throw Wrong(value, "a variant name");
        }
        var at = Array.IndexOf(names, s.Value);
        if (at < 0)
        {
            var expected = names.Length == 1 ? $"expected `{names[0]}`" : $"expected one of {string.Join(", ", names.Select(n => $"`{n}`"))}";
            throw new JsonShapeException($"unknown variant `{s.Value}`, {expected}");
        }
        return at;
    }

    /// <summary>Reads with <paramref name="read"/>, turning a shape problem into an error.</summary>
    public static Result<T, string> Try<T>(Func<T> read)
    {
        try
        {
            return read();
        }
        catch (JsonShapeException e)
        {
            return Result.Fail(e.Message);
        }
    }

    public static string Invariant(FormattableString text) => text.ToString(CultureInfo.InvariantCulture);
}
