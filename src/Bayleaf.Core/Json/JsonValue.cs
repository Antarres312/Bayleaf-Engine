using System.Collections;
using System.Globalization;

namespace Bayleaf.Core.Json;

/// <summary>The kind of a <see cref="JsonValue"/>.</summary>
public enum JsonKind
{
    Null,
    Boolean,
    Number,
    Text,
    Array,
    Map,
}

/// <summary>
/// A JSON value as the engine's files hold it. Numbers keep whether they were whole (positive or
/// negative) or not, as the Rust engine's JSON values do, so a value read and written again comes
/// out the same. Objects keep their entries in order; <see cref="Canonical"/> gives the form the
/// Rust engine's untyped values have (keys sorted, later duplicates winning).
/// </summary>
public abstract class JsonValue : IEquatable<JsonValue>
{
    private protected JsonValue()
    {
    }

    public abstract JsonKind Kind { get; }

    public static JsonValue Null => JsonNull.Instance;

    public static JsonValue From(bool value) => value ? JsonBool.True : JsonBool.False;

    public static JsonValue From(string value) => new JsonString(value);

    public static JsonValue From(double value) => JsonNumber.Of(value);

    public static JsonValue From(float value) => JsonNumber.Of(value);

    public static JsonValue From(long value) => JsonNumber.Of(value);

    public static JsonValue From(ulong value) => JsonNumber.Of(value);

    /// <summary>
    /// This value as an untyped value of the Rust engine holds it: every object's keys in sorted
    /// order (a later duplicate replacing an earlier one), single-precision numbers widened to double.
    /// </summary>
    public abstract JsonValue Canonical();

    public string ToCompact() => JsonWriter.Compact(this);

    public string ToPretty() => JsonWriter.Pretty(this);

    /// <summary>Compact JSON, as the Rust engine shows a value in messages.</summary>
    public override string ToString() => ToCompact();

    /// <summary>Equal as untyped values: numbers of the same kind and value, objects with the same entries.</summary>
    public abstract bool Equals(JsonValue? other);

    public override bool Equals(object? obj) => obj is JsonValue other && Equals(other);

    public abstract override int GetHashCode();

    public bool IsNull => Kind == JsonKind.Null;

    public bool? AsBool() => this is JsonBool b ? b.Value : null;

    public string? AsString() => this is JsonString s ? s.Value : null;

    public double? AsDouble() => this is JsonNumber n ? n.ToDouble() : null;

    public JsonArray? AsArray() => this as JsonArray;

    public JsonObject? AsObject() => this as JsonObject;

    /// <summary>The value under <paramref name="key"/> when this is an object that has it.</summary>
    public JsonValue? Get(string key) => this is JsonObject o ? o.Find(key) : null;
}

public sealed class JsonNull : JsonValue
{
    public static readonly JsonNull Instance = new();

    private JsonNull()
    {
    }

    public override JsonKind Kind => JsonKind.Null;

    public override JsonValue Canonical() => this;

    public override bool Equals(JsonValue? other) => other is JsonNull;

    public override int GetHashCode() => 0;
}

public sealed class JsonBool : JsonValue
{
    public static readonly JsonBool True = new(true);
    public static readonly JsonBool False = new(false);

    private JsonBool(bool value) => Value = value;

    public bool Value { get; }

    public override JsonKind Kind => JsonKind.Boolean;

    public override JsonValue Canonical() => this;

    public override bool Equals(JsonValue? other) => other is JsonBool b && b.Value == Value;

    public override int GetHashCode() => Value ? 1 : 2;
}

public sealed class JsonString(string value) : JsonValue
{
    public string Value { get; } = value;

    public override JsonKind Kind => JsonKind.Text;

    public override JsonValue Canonical() => this;

    public override bool Equals(JsonValue? other) => other is JsonString s && string.Equals(s.Value, Value, StringComparison.Ordinal);

    public override int GetHashCode() => StringComparer.Ordinal.GetHashCode(Value);
}

/// <summary>How a number is kept.</summary>
public enum NumberKind
{
    /// <summary>A whole number of 0 or more.</summary>
    Natural,

    /// <summary>A whole number below 0.</summary>
    NegativeWhole,

    /// <summary>A double-precision number.</summary>
    Real,

    /// <summary>A single-precision number, written in its own shortest form; an untyped value widens it to double.</summary>
    Real32,
}

public sealed class JsonNumber : JsonValue
{
    private readonly ulong unsigned;
    private readonly long signed;
    private readonly double real;

    private JsonNumber(NumberKind kind, ulong unsigned, long signed, double real)
    {
        NumberKind = kind;
        this.unsigned = unsigned;
        this.signed = signed;
        this.real = real;
    }

    public NumberKind NumberKind { get; }

    public override JsonKind Kind => JsonKind.Number;

    public static JsonNumber Of(ulong value) => new(NumberKind.Natural, value, 0, 0);

    /// <summary>A whole number: positive or negative as the Rust engine keeps them.</summary>
    public static JsonNumber Of(long value) => value >= 0 ? new(NumberKind.Natural, (ulong)value, 0, 0) : new(NumberKind.NegativeWhole, 0, value, 0);

    public static JsonNumber Of(double value) => new(NumberKind.Real, 0, 0, value);

    public static JsonNumber Of(float value) => new(NumberKind.Real32, 0, 0, value);

    public bool IsInteger => NumberKind is NumberKind.Natural or NumberKind.NegativeWhole;

    /// <summary>The whole number of 0 or more, when the number is one.</summary>
    public ulong? AsUInt64() => NumberKind == NumberKind.Natural ? unsigned : null;

    /// <summary>The whole number, when it is one that fits.</summary>
    public long? AsInt64() => NumberKind switch
    {
        NumberKind.Natural => unsigned <= long.MaxValue ? (long)unsigned : null,
        NumberKind.NegativeWhole => signed,
        _ => null,
    };

    /// <summary>The number as a double, as the Rust engine reads any number into one.</summary>
    public double ToDouble() => NumberKind switch
    {
        NumberKind.Natural => unsigned,
        NumberKind.NegativeWhole => signed,
        _ => real,
    };

    /// <summary>The number as a float, as the Rust engine reads any number into one.</summary>
    public float ToSingle() => NumberKind switch
    {
        NumberKind.Natural => unsigned,
        NumberKind.NegativeWhole => signed,
        NumberKind.Real32 => (float)real,
        _ => (float)real,
    };

    public override JsonValue Canonical() => NumberKind == NumberKind.Real32 ? Of(real) : this;

    public override bool Equals(JsonValue? other)
    {
        if (other is not JsonNumber n)
        {
            return false;
        }
        var (a, b) = (Canonical() as JsonNumber, n.Canonical() as JsonNumber);
        if (a is null || b is null || a.NumberKind != b.NumberKind)
        {
            return false;
        }
        return a.NumberKind switch
        {
            NumberKind.Natural => a.unsigned == b.unsigned,
            NumberKind.NegativeWhole => a.signed == b.signed,
            _ => a.real.Equals(b.real) || a.real == b.real,
        };
    }

    public override int GetHashCode() => NumberKind switch
    {
        NumberKind.Natural => HashCode.Combine(1, unsigned),
        NumberKind.NegativeWhole => HashCode.Combine(2, signed),
        _ => HashCode.Combine(3, real == 0 ? 0.0 : real),
    };

    /// <summary>The number as the Rust engine writes it.</summary>
    public string Format() => NumberKind switch
    {
        NumberKind.Natural => unsigned.ToString(CultureInfo.InvariantCulture),
        NumberKind.NegativeWhole => signed.ToString(CultureInfo.InvariantCulture),
        NumberKind.Real32 => double.IsFinite(real) ? JsonNumbers.Format((float)real) : "null",
        _ => double.IsFinite(real) ? JsonNumbers.Format(real) : "null",
    };
}

public sealed class JsonArray : JsonValue, IReadOnlyList<JsonValue>
{
    private readonly List<JsonValue> items;

    public JsonArray() => items = [];

    public JsonArray(IEnumerable<JsonValue> items) => this.items = [.. items];

    public override JsonKind Kind => JsonKind.Array;

    public int Count => items.Count;

    public JsonValue this[int index] => items[index];

    public void Add(JsonValue item) => items.Add(item);

    public override JsonValue Canonical() => new JsonArray(items.Select(i => i.Canonical()));

    public override bool Equals(JsonValue? other) => other is JsonArray a && a.items.Count == items.Count && items.Zip(a.items).All(p => p.First.Equals(p.Second));

    public override int GetHashCode()
    {
        var hash = new HashCode();
        foreach (var item in items)
        {
            hash.Add(item.GetHashCode());
        }
        return hash.ToHashCode();
    }

    public IEnumerator<JsonValue> GetEnumerator() => items.GetEnumerator();

    IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();
}

/// <summary>An object's entries, in the order they were added or read (duplicates included, as read).</summary>
public sealed class JsonObject : JsonValue, IEnumerable<KeyValuePair<string, JsonValue>>
{
    private readonly List<KeyValuePair<string, JsonValue>> entries;

    public JsonObject() => entries = [];

    public JsonObject(IEnumerable<KeyValuePair<string, JsonValue>> entries) => this.entries = [.. entries];

    public override JsonKind Kind => JsonKind.Map;

    public int Count => entries.Count;

    public IReadOnlyList<KeyValuePair<string, JsonValue>> Entries => entries;

    public IEnumerable<string> Keys => entries.Select(e => e.Key);

    /// <summary>Adds an entry at the end.</summary>
    public void Add(string key, JsonValue value) => entries.Add(new(key, value));

    /// <summary>Sets the value of <paramref name="key"/>: replaces the last entry of that key, or adds one.</summary>
    public void Set(string key, JsonValue value)
    {
        for (var i = entries.Count - 1; i >= 0; i--)
        {
            if (string.Equals(entries[i].Key, key, StringComparison.Ordinal))
            {
                entries[i] = new(key, value);
                return;
            }
        }
        entries.Add(new(key, value));
    }

    /// <summary>Removes every entry of <paramref name="key"/> and gives the last one's value.</summary>
    public JsonValue? Remove(string key)
    {
        var found = Find(key);
        entries.RemoveAll(e => string.Equals(e.Key, key, StringComparison.Ordinal));
        return found;
    }

    /// <summary>The value of the last entry of <paramref name="key"/>, as an untyped value reads it.</summary>
    public JsonValue? Find(string key)
    {
        for (var i = entries.Count - 1; i >= 0; i--)
        {
            if (string.Equals(entries[i].Key, key, StringComparison.Ordinal))
            {
                return entries[i].Value;
            }
        }
        return null;
    }

    public bool ContainsKey(string key) => Find(key) is not null;

    public override JsonValue Canonical()
    {
        var sorted = new SortedDictionary<string, JsonValue>(Utf8Order.Instance);
        foreach (var (key, value) in entries)
        {
            sorted[key] = value.Canonical();
        }
        return new JsonObject(sorted);
    }

    public override bool Equals(JsonValue? other)
    {
        if (other is not JsonObject o)
        {
            return false;
        }
        var (a, b) = ((JsonObject)Canonical(), (JsonObject)o.Canonical());
        return a.entries.Count == b.entries.Count
            && a.entries.Zip(b.entries).All(p => string.Equals(p.First.Key, p.Second.Key, StringComparison.Ordinal) && p.First.Value.Equals(p.Second.Value));
    }

    public override int GetHashCode()
    {
        var hash = new HashCode();
        foreach (var (key, value) in ((JsonObject)Canonical()).entries)
        {
            hash.Add(StringComparer.Ordinal.GetHashCode(key));
            hash.Add(value.GetHashCode());
        }
        return hash.ToHashCode();
    }

    public IEnumerator<KeyValuePair<string, JsonValue>> GetEnumerator() => entries.GetEnumerator();

    IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();
}
