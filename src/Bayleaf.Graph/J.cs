using System.Globalization;
using System.Numerics;

using Bayleaf.Core.Json;

namespace Bayleaf.Graph;

/// <summary>Short writers and readers of the values graph files hold.</summary>
internal static class J
{
    public static JsonValue F(float value) => JsonNumber.Of(value);

    public static JsonValue D(double value) => JsonNumber.Of(value);

    public static JsonValue S(string value) => new JsonString(value);

    public static JsonValue B(bool value) => JsonValue.From(value);

    public static JsonValue U(uint value) => JsonNumber.Of((ulong)value);

    public static JsonArray V2(Vector2 v) => new([F(v.X), F(v.Y)]);

    public static JsonArray Strings(IEnumerable<string> values) => new(values.Select(S));

    public static Vector2 ReadV2(JsonValue value)
    {
        var a = JsonRead.Fixed(value, 2, JsonRead.F32);
        return new(a[0], a[1]);
    }

    /// <summary>An untyped value as the Rust engine keeps one: keys sorted, single precision widened.</summary>
    public static JsonValue Untyped(JsonValue value) => value.Canonical();

    /// <summary>A number of any kind as a double (<c>as_f64</c>), or none.</summary>
    public static double? AsF64(JsonValue value) => value is JsonNumber n ? n.ToDouble() : null;

    /// <summary>A whole number that fits a 64-bit integer (<c>as_i64</c>), or none.</summary>
    public static long? AsI64(JsonValue value) => value is JsonNumber n ? n.AsInt64() : null;

    /// <summary>A finite number below single precision's largest, as values and settings take one.</summary>
    public static bool IsNumber(JsonValue value) => AsF64(value) is { } f && double.IsFinite(f) && Math.Abs(f) < float.MaxValue;

    /// <summary>Three such numbers.</summary>
    public static bool IsTriple(JsonValue value) => value is JsonArray { Count: 3 } a && a.All(IsNumber);

    /// <summary>Text parsed as the Rust engine parses an <c>f32</c>: no white space around it.</summary>
    public static float? ParseF32(string text) =>
        text.Length > 0 && float.TryParse(text, NumberStyles.AllowLeadingSign | NumberStyles.AllowDecimalPoint | NumberStyles.AllowExponent, CultureInfo.InvariantCulture, out var v) ? v : null;

    /// <summary>Text parsed as the Rust engine parses an <c>i32</c>.</summary>
    public static int? ParseI32(string text) => int.TryParse(text, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var v) ? v : null;

    /// <summary>Whether a name is 1 to 64 ASCII letters, digits or <c>_</c>.</summary>
    public static bool IsName(string text) => text.Length is > 0 and <= 64 && text.All(c => char.IsAsciiLetterOrDigit(c) || c == '_');
}
