using System.Numerics;

namespace Bayleaf.Animation;

/// <summary>Single-precision helpers that work as the Rust engine's do.</summary>
internal static class Numbers
{
    /// <summary>The gap between 1 and the next single-precision number (Rust's <c>f32::EPSILON</c>).</summary>
    public const float MachineEpsilon = 1.1920929e-7f;

    /// <summary>Orders numbers as Rust's <c>total_cmp</c> does (negative NaN first, positive NaN last).</summary>
    public static readonly IComparer<float> TotalOrder = Comparer<float>.Create(TotalCmp);

    public static int TotalCmp(float a, float b)
    {
        static int Key(float v)
        {
            var bits = BitConverter.SingleToInt32Bits(v);
            return bits ^ (int)((uint)(bits >> 31) >> 1);
        }
        return Key(a).CompareTo(Key(b));
    }

    /// <summary>The larger of two numbers, the other one when either is NaN (Rust's <c>f32::max</c>).</summary>
    public static float Max(float a, float b) => float.IsNaN(a) ? b : float.IsNaN(b) ? a : a > b ? a : b;

    /// <summary>The remainder of <paramref name="a"/> over <paramref name="b"/>, never negative (Rust's <c>rem_euclid</c>).</summary>
    public static float RemEuclid(float a, float b)
    {
        var r = a % b;
        return r < 0 ? r + MathF.Abs(b) : r;
    }

    public static Quaternion Normalize(Quaternion q)
    {
        var recip = 1f / MathF.Sqrt(q.X * q.X + q.Y * q.Y + q.Z * q.Z + q.W * q.W);
        return new(q.X * recip, q.Y * recip, q.Z * recip, q.W * recip);
    }

    /// <summary>The index of the largest number, the last of equal ones (Rust's <c>max_by</c> with <c>total_cmp</c>); null for none.</summary>
    public static int? IndexOfMax(IReadOnlyList<float> values)
    {
        int? best = null;
        for (var i = 0; i < values.Count; i++)
        {
            if (best is not { } b || TotalCmp(values[i], values[b]) >= 0)
            {
                best = i;
            }
        }
        return best;
    }

    public static bool IsFinite(Vector3 v) => float.IsFinite(v.X) && float.IsFinite(v.Y) && float.IsFinite(v.Z);

    public static bool IsFinite(Matrix4x4 m) =>
        float.IsFinite(m.M11) && float.IsFinite(m.M12) && float.IsFinite(m.M13) && float.IsFinite(m.M14)
        && float.IsFinite(m.M21) && float.IsFinite(m.M22) && float.IsFinite(m.M23) && float.IsFinite(m.M24)
        && float.IsFinite(m.M31) && float.IsFinite(m.M32) && float.IsFinite(m.M33) && float.IsFinite(m.M34)
        && float.IsFinite(m.M41) && float.IsFinite(m.M42) && float.IsFinite(m.M43) && float.IsFinite(m.M44);
}
