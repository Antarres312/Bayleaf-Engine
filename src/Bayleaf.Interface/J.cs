using System.Globalization;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.GameInterface;

/// <summary>Short writers, readers and number rules of the interface's files and layout.</summary>
internal static class J
{
    public static JsonValue S(string value) => new JsonString(value);

    public static JsonValue D(double value) => JsonNumber.Of(value);

    public static JsonValue U(uint value) => JsonNumber.Of((ulong)value);

    public static JsonArray Strings(IEnumerable<string> values) => new(values.Select(S));

    public static JsonObject Map(SortedDictionary<string, string> values) => new(values.Select(p => new KeyValuePair<string, JsonValue>(p.Key, S(p.Value))));

    public static SortedDictionary<string, string> ReadMap(JsonValue value) => JsonRead.Map(value, JsonRead.Str);

    public static SortedDictionary<string, string> EmptyMap() => new(Utf8Order.Instance);

    /// <summary>Text parsed as the Rust engine parses an <c>f32</c>: no white space around it.</summary>
    public static float? ParseF32(string text) =>
        text.Length > 0 && float.TryParse(text, NumberStyles.AllowLeadingSign | NumberStyles.AllowDecimalPoint | NumberStyles.AllowExponent, CultureInfo.InvariantCulture, out var v) ? v : null;

    /// <summary>The larger number; a NaN gives the other, as the Rust engine's <c>max</c> does.</summary>
    public static float Max(float a, float b) => float.IsNaN(a) ? b : float.IsNaN(b) ? a : Math.Max(a, b);

    /// <summary>The smaller number; a NaN gives the other, as the Rust engine's <c>min</c> does.</summary>
    public static float Min(float a, float b) => float.IsNaN(a) ? b : float.IsNaN(b) ? a : Math.Min(a, b);

    /// <summary>A sum in order, from negative zero as the Rust engine sums.</summary>
    public static float Sum(IEnumerable<float> values)
    {
        var sum = -0f;
        foreach (var v in values)
        {
            sum += v;
        }
        return sum;
    }

    public static string Debug(string text) => Text.Debug(text);

    public static Result<Unit, string> Ok => Result.Ok<string>();
}
