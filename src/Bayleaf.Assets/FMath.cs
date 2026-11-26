using System.Numerics;

namespace Bayleaf.Assets;

/// <summary>Single-precision helpers that give the Rust engine's results: its rounding, its saturating casts, its min and max.</summary>
internal static class FMath
{
    /// <summary>The gap between 1 and the next float (Rust's <c>f32::EPSILON</c>; .NET's <c>float.Epsilon</c> is the smallest float instead).</summary>
    public const float MachineEpsilon = 1.1920929e-7f;

    /// <summary>The smaller, ignoring a NaN as Rust's <c>f32::min</c> does.</summary>
    public static float Min(float a, float b) => float.IsNaN(a) ? b : float.IsNaN(b) ? a : a < b ? a : b;

    /// <summary>The larger, ignoring a NaN as Rust's <c>f32::max</c> does.</summary>
    public static float Max(float a, float b) => float.IsNaN(a) ? b : float.IsNaN(b) ? a : a > b ? a : b;

    public static Vector3 Min(Vector3 a, Vector3 b) => new(Min(a.X, b.X), Min(a.Y, b.Y), Min(a.Z, b.Z));

    public static Vector3 Max(Vector3 a, Vector3 b) => new(Max(a.X, b.X), Max(a.Y, b.Y), Max(a.Z, b.Z));

    /// <summary>Rounded half away from zero, as Rust's <c>f32::round</c>.</summary>
    public static float Round(float v) => MathF.Round(v, MidpointRounding.AwayFromZero);

    /// <summary>Clamped to <paramref name="low"/>..<paramref name="high"/>; NaN stays NaN, as Rust's <c>f32::clamp</c>.</summary>
    public static float Clamp(float v, float low, float high) => float.IsNaN(v) ? v : v < low ? low : v > high ? high : v;

    /// <summary>A float as a byte the way Rust's <c>as u8</c> casts it: saturating, NaN as 0.</summary>
    public static byte ToByte(float v) => float.IsNaN(v) ? (byte)0 : v <= 0 ? (byte)0 : v >= 255 ? (byte)255 : (byte)v;

    /// <summary>A float as a 16-bit number the way Rust's <c>as u16</c> casts it.</summary>
    public static ushort ToUInt16(float v) => float.IsNaN(v) ? (ushort)0 : v <= 0 ? (ushort)0 : v >= 65535 ? (ushort)65535 : (ushort)v;

    public static float SrgbToLinear(float v) => v <= 0.04045f ? v / 12.92f : MathF.Pow((v + 0.055f) / 1.055f, 2.4f);

    public static float LinearToSrgb(float v)
    {
        v = Clamp(v, 0f, 1f);
        return v <= 0.0031308f ? v * 12.92f : 1.055f * MathF.Pow(v, 1f / 2.4f) - 0.055f;
    }

    /// <summary>The remainder of <paramref name="a"/> over <paramref name="b"/>, never negative (Rust's <c>rem_euclid</c>).</summary>
    public static float RemEuclid(float a, float b)
    {
        var r = a % b;
        return r < 0 ? r + MathF.Abs(b) : r;
    }

    /// <summary>The vector at length 1, or zero when it has no finite length.</summary>
    public static Vector3 NormalizeOrZero(Vector3 v) => NormalizeOr(v, Vector3.Zero);

    /// <summary>The vector at length 1, or <paramref name="fallback"/> when it has no finite length.</summary>
    public static Vector3 NormalizeOr(Vector3 v, Vector3 fallback)
    {
        var recip = 1f / MathF.Sqrt(v.X * v.X + v.Y * v.Y + v.Z * v.Z);
        return float.IsFinite(recip) && recip > 0 ? v * recip : fallback;
    }

    public static Quaternion Normalize(Quaternion q)
    {
        var recip = 1f / MathF.Sqrt(q.X * q.X + q.Y * q.Y + q.Z * q.Z + q.W * q.W);
        return new(q.X * recip, q.Y * recip, q.Z * recip, q.W * recip);
    }

    /// <summary>From <paramref name="a"/> to <paramref name="b"/> along the shorter arc.</summary>
    public static Quaternion Slerp(Quaternion a, Quaternion b, float t)
    {
        var dot = a.X * b.X + a.Y * b.Y + a.Z * b.Z + a.W * b.W;
        if (dot < 0)
        {
            b = new(-b.X, -b.Y, -b.Z, -b.W);
            dot = -dot;
        }
        if (dot > 1f - MachineEpsilon)
        {
            // Nearly the same turn: a straight line, made unit length again.
            return Normalize(new(a.X + (b.X - a.X) * t, a.Y + (b.Y - a.Y) * t, a.Z + (b.Z - a.Z) * t, a.W + (b.W - a.W) * t));
        }
        var theta = MathF.Acos(dot);
        var (s1, s2, sin) = (MathF.Sin(theta * (1 - t)), MathF.Sin(theta * t), MathF.Sin(theta));
        var r = 1f / sin;
        return new((a.X * s1 + b.X * s2) * r, (a.Y * s1 + b.Y * s2) * r, (a.Z * s1 + b.Z * s2) * r, (a.W * s1 + b.W * s2) * r);
    }

    /// <summary>Scales, then turns, then moves: a point is <c>Vector3.Transform(p, m)</c>.</summary>
    public static Matrix4x4 FromScaleRotationTranslation(Vector3 scale, Quaternion rotation, Vector3 translation)
    {
        var m = Matrix4x4.CreateFromQuaternion(rotation);
        return new(
            m.M11 * scale.X, m.M12 * scale.X, m.M13 * scale.X, 0,
            m.M21 * scale.Y, m.M22 * scale.Y, m.M23 * scale.Y, 0,
            m.M31 * scale.Z, m.M32 * scale.Z, m.M33 * scale.Z, 0,
            translation.X, translation.Y, translation.Z, 1);
    }

    /// <summary>A 4 x 4 matrix of glTF's (or glam's) column-major numbers, as System.Numerics keeps it (each column a row).</summary>
    public static Matrix4x4 FromColumnMajor(ReadOnlySpan<float> m) => new(
        m[0], m[1], m[2], m[3],
        m[4], m[5], m[6], m[7],
        m[8], m[9], m[10], m[11],
        m[12], m[13], m[14], m[15]);

    /// <summary>The inverse; every entry NaN when there is none (as the Rust engine's inverse then gives no finite numbers).</summary>
    public static Matrix4x4 Inverse(Matrix4x4 m) => Matrix4x4.Invert(m, out var inverse) ? inverse : NaN;

    private static readonly Matrix4x4 NaN = new(float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN,
        float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN, float.NaN);

    /// <summary>The matrix normals move by under <paramref name="m"/>: the inverse transpose of its 3 x 3 part, for <c>Vector3.TransformNormal</c>.</summary>
    public static Matrix4x4 NormalMatrix(Matrix4x4 m)
    {
        var linear = new Matrix4x4(m.M11, m.M12, m.M13, 0, m.M21, m.M22, m.M23, 0, m.M31, m.M32, m.M33, 0, 0, 0, 0, 1);
        return Matrix4x4.Transpose(Inverse(linear));
    }

    /// <summary>The 3 x 3 part's determinant.</summary>
    public static float Determinant3(Matrix4x4 m) =>
        m.M11 * (m.M22 * m.M33 - m.M23 * m.M32) - m.M12 * (m.M21 * m.M33 - m.M23 * m.M31) + m.M13 * (m.M21 * m.M32 - m.M22 * m.M31);

    public static Matrix4x4 Scale(Matrix4x4 m, float s) => new(
        m.M11 * s, m.M12 * s, m.M13 * s, m.M14 * s,
        m.M21 * s, m.M22 * s, m.M23 * s, m.M24 * s,
        m.M31 * s, m.M32 * s, m.M33 * s, m.M34 * s,
        m.M41 * s, m.M42 * s, m.M43 * s, m.M44 * s);
}
