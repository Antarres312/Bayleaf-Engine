using System.Numerics;

using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;

namespace Bayleaf.Scene;

/// <summary>Short writers and readers of the values scene files hold.</summary>
internal static class J
{
    public static JsonValue F(float value) => JsonNumber.Of(value);

    public static JsonValue D(double value) => JsonNumber.Of(value);

    public static JsonValue S(string value) => new JsonString(value);

    public static JsonValue B(bool value) => JsonValue.From(value);

    public static JsonValue U(uint value) => JsonNumber.Of((ulong)value);

    public static JsonValue I(int value) => JsonNumber.Of((long)value);

    public static JsonArray Floats(params float[] values) => new(values.Select(F));

    public static JsonArray Doubles(params double[] values) => new(values.Select(D));

    public static JsonArray V2(Vector2 v) => Floats(v.X, v.Y);

    public static JsonArray V3(Vector3 v) => Floats(v.X, v.Y, v.Z);

    public static JsonArray V4(Vector4 v) => Floats(v.X, v.Y, v.Z, v.W);

    public static JsonArray D3(Double3 v) => Doubles(v.X, v.Y, v.Z);

    public static JsonArray Strings(IEnumerable<string> values) => new(values.Select(S));

    public static Vector2 ReadV2(JsonValue value)
    {
        var a = JsonRead.Fixed(value, 2, JsonRead.F32);
        return new(a[0], a[1]);
    }

    public static Vector3 ReadV3(JsonValue value)
    {
        var a = JsonRead.Fixed(value, 3, JsonRead.F32);
        return new(a[0], a[1], a[2]);
    }

    public static Vector4 ReadV4(JsonValue value)
    {
        var a = JsonRead.Fixed(value, 4, JsonRead.F32);
        return new(a[0], a[1], a[2], a[3]);
    }

    public static Double3 ReadD3(JsonValue value) => Double3.FromArray(JsonRead.Fixed(value, 3, JsonRead.F64));

    public static float[] Of(Vector2 v) => [v.X, v.Y];

    public static float[] Of(Vector3 v) => [v.X, v.Y, v.Z];

    public static float[] Of(Vector4 v) => [v.X, v.Y, v.Z, v.W];

    public static bool Finite(float v) => float.IsFinite(v);

    /// <summary>Whether <paramref name="v"/> is from <paramref name="low"/> to <paramref name="high"/>, both included (never for NaN).</summary>
    public static bool Within(float v, float low, float high) => v >= low && v <= high;

    public static bool Within(double v, double low, double high) => v >= low && v <= high;
}
